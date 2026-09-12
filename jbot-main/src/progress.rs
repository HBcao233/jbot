use std::fmt::Write;
use std::iter::repeat;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use grammers_client::message::Message;
use tokio::io::{self, AsyncRead, ReadBuf};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

const UPDATE_LIMIT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProgressStyle {
    /// {p} / {total}
    Plain,
    /// 百分比，如 `45.67%`
    #[default]
    Percent,
    /// 文件大小，如 `1.23 MB / 10.00 MB`
    Size,
    /// 时间，如 `05:00 / 01:05:45`
    Time,
}

struct Inner {
    p: usize,
    total: usize,
    prefix: String,
    style: ProgressStyle,
    last_update: Instant,
    last_text: Option<String>,
}

pub struct Progress {
    mid: Arc<Message>,
    inner: RwLock<Inner>,
}

impl Progress {
    const CHARS: [char; 9] = [
        '\u{3000}', // 全角空格
        '\u{258f}', // 1/8 方块
        '\u{258e}', // 2/8 方块
        '\u{258d}', // 3/8 方块
        '\u{258c}', // 4/8 方块
        '\u{258b}', // 5/8 方块
        '\u{258a}', // 6/8 方块
        '\u{2589}', // 7/8 方块
        '\u{2588}', // 全方块
    ];

    pub fn new(mid: Arc<Message>) -> Self {
        let now = Instant::now();
        Self {
            mid,
            inner: RwLock::new(Inner {
                p: 0,
                total: 100,
                prefix: String::new(),
                style: ProgressStyle::Percent,
                last_update: now.checked_sub(UPDATE_LIMIT).unwrap_or(now),
                last_text: None,
            }),
        }
    }

    pub fn total(&self, total: usize) {
        self.inner.write().unwrap().total = total;
    }

    pub fn prefix(&self, prefix: &str) {
        self.inner.write().unwrap().prefix = prefix.to_string();
    }

    pub fn style(&self, style: ProgressStyle) {
        self.inner.write().unwrap().style = style;
    }

    fn format_size(bytes: usize) -> String {
        const KB: f64 = 1024.0;
        const MB: f64 = KB * 1024.0;
        const GB: f64 = MB * 1024.0;

        let n = bytes as f64;
        if n >= GB {
            format!("{:.2} GB", n / GB)
        } else if n >= MB {
            format!("{:.2} MB", n / MB)
        } else if n >= KB {
            format!("{:.2} KB", n / KB)
        } else {
            format!("{bytes} B")
        }
    }

    fn format_time(secs: usize) -> String {
        let hours = secs / 3600;
        let minutes = (secs % 3600) / 60;
        let seconds = secs % 60;

        if hours > 0 {
            format!("{hours:02}:{minutes:02}:{seconds:02}")
        } else {
            format!("{minutes:02}:{seconds:02}")
        }
    }

    fn get_text(p: usize, total: usize, style: ProgressStyle, prefix: &str) -> String {
        const CELLS: usize = 13;
        const STEPS: usize = CELLS * 8;

        let p = p.min(total);
        let x = if total == 0 {
            0
        } else {
            (p as u64 * STEPS as u64 / total as u64) as usize
        };
        let (full, part) = (x / 8, x % 8);

        let mut text = String::with_capacity(prefix.len() + CELLS * 3 + 48);
        text.push_str(prefix);
        text.push('[');
        text.extend(repeat(Self::CHARS[8]).take(full));
        if x != STEPS {
            text.push(Self::CHARS[part]);
            text.extend(repeat(Self::CHARS[0]).take(CELLS - full - 1));
        }
        text.push(']');
        text.push(' ');
        match style {
            ProgressStyle::Plain => write!(text, "{} / {}", p, total).unwrap(),
            ProgressStyle::Percent => {
                if total == 0 {
                    text.push_str("0.00%");
                } else {
                    write!(text, "{:.2}%", (p as f64 / total as f64) * 100.0).unwrap();
                }
            }
            ProgressStyle::Size => {
                write!(
                    text,
                    "{} / {}",
                    Self::format_size(p),
                    Self::format_size(total)
                )
                .unwrap();
            }
            ProgressStyle::Time => {
                write!(
                    text,
                    "{} / {}",
                    Self::format_time(p),
                    Self::format_time(total)
                )
                .unwrap();
            }
        }

        text
    }

    pub async fn update(&self, p: usize, total: Option<usize>) {
        {
            let mut guard = self.inner.write().unwrap();
            guard.p = p;
            if let Some(t) = total {
                guard.total = t;
            }
        }
        self.update_display().await;
    }

    pub async fn add(&self, add: usize) {
        {
            let mut guard = self.inner.write().unwrap();
            guard.p = guard.p.saturating_add(add);
        }
        self.update_display().await;
    }

    async fn update_display(&self) {
        let (p, total, style, prefix, last_text, last_update) = {
            let guard = self.inner.read().unwrap();
            (
                guard.p,
                guard.total,
                guard.style,
                guard.prefix.clone(),
                guard.last_text.clone(),
                guard.last_update,
            )
        };
        if last_update.elapsed() < UPDATE_LIMIT {
            return;
        }

        let text = Self::get_text(p, total, style, &prefix);
        if Some(&text) == last_text.as_ref() {
            let mut guard = self.inner.write().unwrap();
            guard.last_update = Instant::now();
            return;
        }

        match self.mid.edit(&text).await {
            Ok(_) => {
                let mut guard = self.inner.write().unwrap();
                guard.last_update = Instant::now();
                guard.last_text = Some(text);
            }
            Err(e) => {
                log::error!("进度条更新失败: {e}");
            }
        }
    }
}

pub struct ProgressScheduler {
    bar: Arc<Progress>,
    tx: mpsc::Sender<(usize, Option<usize>)>,
    task: JoinHandle<()>,
}

impl ProgressScheduler {
    pub fn new(bar: Progress) -> Self {
        let bar = Arc::new(bar);
        let (tx, mut rx) = mpsc::channel(10);
        let bar_clone = bar.clone();

        Self {
            bar,
            task: tokio::spawn(async move {
                while let Some((downloaded, total)) = rx.recv().await {
                    bar_clone.update(downloaded, total).await;
                }
            }),
            tx,
        }
    }

    pub fn sync_update(&self, downloaded: usize, total: Option<usize>) {
        let _ = self.tx.try_send((downloaded, total));
    }
}

impl std::ops::Deref for ProgressScheduler {
    type Target = Progress;

    fn deref(&self) -> &Self::Target {
        &self.bar
    }
}

impl Drop for ProgressScheduler {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub struct ProgressReader<T, F> {
    inner: T,
    callback: F,
    uploaded: usize,
    total: Option<usize>,
}

impl<T, F> ProgressReader<T, F> {
    pub fn new(inner: T, callback: F, total: Option<usize>) -> Self {
        Self {
            inner,
            callback,
            uploaded: 0,
            total,
        }
    }
}

impl<T, F> AsyncRead for ProgressReader<T, F>
where
    T: AsyncRead + Unpin,
    F: Fn(usize, Option<usize>) + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();

        let me = self.get_mut();
        match Pin::new(&mut me.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                let after = buf.filled().len();
                let bytes_read = after - before;

                if bytes_read > 0 {
                    me.uploaded += bytes_read;
                    (me.callback)(me.uploaded, me.total);
                }

                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}
