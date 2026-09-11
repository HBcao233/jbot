use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use wreq_util::Emulation::Chrome137;

pub fn get_client() -> wreq::ClientBuilder {
    wreq::Client::builder().emulation(Chrome137)
}

pub async fn stream_download(
    client: &wreq::Client,
    url: String,
    name: &str,
    headers: &[(&str, String)],
) -> anyhow::Result<PathBuf> {
    let cache_dir = Path::new("cache");
    if let Err(e) = fs::create_dir_all(cache_dir).await {
        log::error!("缓存文件夹创建失败: {e:?}");
    }

    let path = cache_dir.join(name);
    if path.is_file() {
        return Ok(path);
    }

    let mut request = client.get(url);
    for (k, v) in headers {
        request = request.header(k.to_string(), v);
    }
    let response = request.send().await?;

    let status = response.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!(format!(
            "下载失败，HTTP 状态码：{}",
            status,
        )));
    }

    let mut stream = response.bytes_stream();
    let mut file = fs::File::create(&path).await?;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;

        file.write_all(&chunk).await?;
    }

    file.flush().await?;

    Ok(path)
}
