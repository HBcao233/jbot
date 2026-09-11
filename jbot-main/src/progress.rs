use std::iter::repeat;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use grammers_client::message::Message;
use tokio::io::{self, AsyncRead, ReadBuf};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

const UPDATE_LIMIT: Duration = Duration::from_secs(1);

struct Inner {
    p: usize,
    total: usize,
    prefix: String,
    percent: bool,
    last_update: Instant,
}

pub struct Progress {
    mid: Arc<Message>,
    inner: Mutex<Inner>,
}

impl Progress {
    const CHARS: [char; 9] = [
        '\u{3000}', // 全角空格
        '\u{258f}', // 1/8 方块
        '\u{258e}', '\u{258d}', '\u{258c}', '\u{258b}', '\u{258a}', '\u{2589}',
        '\u{2588}', // 全方块
    ];

    pub fn new(mid: Arc<Message>) -> Self {
        let now = Instant::now();
        Self {
            mid,
            inner: Mutex::new(Inner {
                p: 0,
                total: 100,
                prefix: String::new(),
                percent: true,
                last_update: now.checked_sub(UPDATE_LIMIT).unwrap_or(now),
            }),
        }
    }

    pub fn total(&self, total: usize) {
        self.inner.lock().unwrap().total = total;
    }

    pub fn prefix(&self, prefix: &str) {
        self.inner.lock().unwrap().prefix = prefix.to_string();
    }

    pub fn percent(&self, percent: bool) {
        self.inner.lock().unwrap().percent = percent;
    }

    fn get_text(p: usize, total: usize, percent: bool) -> String {
        const CELLS: usize = 13;
        const STEPS: usize = CELLS * 8;

        let p = p.min(total);
        let x = if total == 0 {
            0
        } else {
            (p as u64 * STEPS as u64 / total as u64) as usize
        };
        let (full, part) = (x / 8, x % 8);

        let mut text = String::with_capacity(30);
        text.extend(repeat(Self::CHARS[8]).take(full));
        if x != STEPS {
            text.push(Self::CHARS[part]);
            text.extend(repeat(Self::CHARS[0]).take(CELLS - full - 1));
        }
        let suffix = if percent {
            if total == 0 {
                "0.00%".to_string()
            } else {
                format!("{:.2}%", (p as f64 / total as f64) * 100.0)
            }
        } else {
            format!("{} / {}", p, total)
        };
        format!("[{}] {}", text, suffix)
    }

    pub async fn update(&self, p: usize, total: Option<usize>) {
        let text = {
            let mut guard = self.inner.lock().unwrap();
            guard.p = p;
            if let Some(t) = total {
                guard.total = t;
            }
            if guard.last_update.elapsed() < UPDATE_LIMIT {
                return;
            }

            let total = guard.total;
            let text = Self::get_text(p, total, guard.percent);
            format!("{}{}", &guard.prefix, text)
        };

        match self.mid.edit(text).await {
            Ok(_) => {
                let mut guard = self.inner.lock().unwrap();
                guard.last_update = Instant::now();
            }
            Err(e) => {
                log::error!("进度条更新失败: {e}");
            }
        }
    }

    pub async fn add(&self, add: usize) {
        let p = self.inner.lock().unwrap().p;
        self.update(p + add, None).await;
    }
}

pub struct ProgressScheduler {
    bar: Arc<Progress>,
    task: JoinHandle<()>,
    tx: mpsc::Sender<(usize, Option<usize>)>,
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
