use crate::arxiv::Paper;
use crate::db::data_dir;
use futures_util::StreamExt;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

const USER_AGENT: &str = "ArxivLibrary/1.0 (cross-platform; personal research tool)";

fn pdf_dir() -> PathBuf {
    let dir = data_dir().join("PDFs");
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// Progress payload emitted to the frontend as bytes arrive.
#[derive(Clone, serde::Serialize)]
struct DownloadProgress {
    arxiv_id: String,
    received: u64,
    total: u64, // 0 when the server doesn't report Content-Length
    done: bool,
}

/// Streams a PDF from `url` to `dest`, emitting "download-progress" events on
/// `app` (if provided) tagged with `arxiv_id`. Returns the saved path.
async fn stream_to_file(
    url: &str,
    dest: &PathBuf,
    arxiv_id: &str,
    app: Option<&AppHandle>,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| format!("Download error: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("PDF download HTTP {}", resp.status().as_u16()));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut received: u64 = 0;
    let mut buf: Vec<u8> = Vec::with_capacity(total.max(1024) as usize);
    let mut stream = resp.bytes_stream();

    // Throttle progress emissions so we don't flood the IPC bridge: emit at most
    // every ~64KB or whenever a chunk arrives, whichever is coarser.
    let mut last_emit: u64 = 0;
    let emit = |arxiv_id: &str, received: u64, total: u64, done: bool| {
        if let Some(app) = app {
            let _ = app.emit(
                "download-progress",
                DownloadProgress {
                    arxiv_id: arxiv_id.to_string(),
                    received,
                    total,
                    done,
                },
            );
        }
    };

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Read error: {e}"))?;
        received += chunk.len() as u64;
        buf.extend_from_slice(&chunk);
        if received - last_emit >= 65_536 {
            last_emit = received;
            emit(arxiv_id, received, total, false);
        }
    }

    std::fs::write(dest, &buf).map_err(|e| format!("Write error: {e}"))?;
    emit(arxiv_id, received, total.max(received), true);
    Ok(dest.to_string_lossy().to_string())
}

/// Downloads the paper's PDF and returns the absolute path as a string.
pub async fn download(paper: &Paper, app: Option<&AppHandle>) -> Result<String, String> {
    let filename = format!("{}.pdf", paper.arxiv_id.replace('/', "_"));
    let dest = pdf_dir().join(&filename);

    if dest.exists() {
        return Ok(dest.to_string_lossy().to_string());
    }
    stream_to_file(&paper.pdf_url, &dest, &paper.arxiv_id, app).await
}

pub fn delete(path: &str) {
    std::fs::remove_file(path).ok();
}

/// Downloads the paper's PDF into the OS temp directory and returns its path.
/// Used for "view without saving to library" — the file is a throwaway.
pub async fn download_to_temp(paper: &Paper, app: Option<&AppHandle>) -> Result<String, String> {
    let filename = format!("arxiv_{}.pdf", paper.arxiv_id.replace('/', "_"));
    let dest = std::env::temp_dir().join(&filename);

    if dest.exists() {
        return Ok(dest.to_string_lossy().to_string());
    }
    stream_to_file(&paper.pdf_url, &dest, &paper.arxiv_id, app).await
}
