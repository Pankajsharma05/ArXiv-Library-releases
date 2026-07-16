use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const ENDPOINT: &str = "https://export.arxiv.org/api/query";
const MIN_INTERVAL: Duration = Duration::from_secs(3);
const USER_AGENT: &str = "ArxivLibrary/1.0 (cross-platform; personal research tool)";

/// Global throttle: arXiv asks for no more than one request every 3 seconds.
static LAST_REQUEST: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paper {
    pub arxiv_id: String,
    pub title: String,
    pub summary: String,
    pub authors: Vec<String>,
    pub categories: Vec<String>,
    pub primary_category: String,
    pub published: String, // ISO8601
    pub updated: String,
    pub pdf_url: String,
    pub abs_url: String,
    #[serde(default)]
    pub doi: Option<String>,
    #[serde(default)]
    pub journal_ref: Option<String>,
    #[serde(default)]
    pub local_pdf_path: Option<String>,
    #[serde(default)]
    pub note: String,
    #[serde(default = "default_status")]
    pub reading_status: String,
    #[serde(default)]
    pub last_opened: Option<String>,
    #[serde(default)]
    pub trashed: bool,
}

fn default_status() -> String { "unread".to_string() }

pub async fn throttle() {
    let mut last = LAST_REQUEST.lock().await;
    if let Some(t) = *last {
        let elapsed = t.elapsed();
        if elapsed < MIN_INTERVAL {
            tokio::time::sleep(MIN_INTERVAL - elapsed).await;
        }
    }
    *last = Some(Instant::now());
}

pub async fn search(
    query: &str,
    start: u32,
    max_results: u32,
    sort_by: &str,
) -> Result<Vec<Paper>, String> {
    throttle().await;
    // "submittedDateOldest" is our virtual sort: submittedDate ascending.
    let (sb, order) = if sort_by == "submittedDateOldest" {
        ("submittedDate", "ascending")
    } else {
        (sort_by, "descending")
    };
    let url = format!(
        "{ENDPOINT}?search_query={}&start={start}&max_results={max_results}&sortBy={sb}&sortOrder={order}",
        urlencoding(query)
    );
    fetch_and_parse(&url).await
}

#[allow(dead_code)]
pub async fn fetch_by_ids(ids: &[String]) -> Result<Vec<Paper>, String> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    throttle().await;
    let url = format!(
        "{ENDPOINT}?id_list={}&max_results={}",
        ids.join(","),
        ids.len()
    );
    fetch_and_parse(&url).await
}

async fn fetch_and_parse(url: &str) -> Result<Vec<Paper>, String> {
    let client = reqwest::Client::new();
    // arXiv's API intermittently returns 503 (load shedding) and 429 (rate limit).
    // Retry a few times with increasing backoff before giving up.
    let mut last_status = 0u16;
    for attempt in 0..4u32 {
        if attempt > 0 {
            // 1s, 2s, 4s backoff.
            let wait = 1000u64 * (1 << (attempt - 1));
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
        }
        let resp = match client
            .get(url)
            .header("User-Agent", USER_AGENT)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                last_status = 0;
                // Network error — retry.
                if attempt < 3 { continue; }
                return Err(format!("Network error: {e}"));
            }
        };
        let status = resp.status();
        if status.is_success() {
            let body = resp.text().await.map_err(|e| format!("Read error: {e}"))?;
            return parse_atom(&body);
        }
        last_status = status.as_u16();
        // Retry on transient server errors; fail fast on client errors (4xx except 429).
        let transient = status.as_u16() == 503 || status.as_u16() == 429 || status.is_server_error();
        if !transient {
            return Err(format!("arXiv returned HTTP {}", last_status));
        }
    }
    Err(format!("arXiv is busy (HTTP {last_status}). Please try again in a moment."))
}

/// Minimal percent-encoding for the query string (spaces, +, &, etc.).
fn urlencoding(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

fn parse_atom(xml: &str) -> Result<Vec<Paper>, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut papers = Vec::new();
    let mut buf = Vec::new();

    let mut in_entry = false;
    let mut in_author = false;
    let mut current_tag = String::new();

    let mut id = String::new();
    let mut title = String::new();
    let mut summary = String::new();
    let mut published = String::new();
    let mut updated = String::new();
    let mut primary_cat = String::new();
    let mut authors: Vec<String> = Vec::new();
    let mut categories: Vec<String> = Vec::new();
    let mut pdf_url: Option<String> = None;
    let mut abs_url: Option<String> = None;
    let mut doi: Option<String> = None;
    let mut journal_ref: Option<String> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(format!("XML parse error: {e}")),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref());
                current_tag = name.clone();
                match name.as_str() {
                    "entry" => {
                        in_entry = true;
                        id.clear(); title.clear(); summary.clear();
                        published.clear(); updated.clear(); primary_cat.clear();
                        authors.clear(); categories.clear();
                        pdf_url = None; abs_url = None;
                        doi = None; journal_ref = None;
                    }
                    "author" if in_entry => in_author = true,
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                // category, link, arxiv:primary_category arrive as empty elements
                let name = local_name(e.name().as_ref());
                if !in_entry {
                    continue;
                }
                match name.as_str() {
                    "category" => {
                        if let Some(term) = attr(&e, "term") {
                            categories.push(term);
                        }
                    }
                    "primary_category" => {
                        if let Some(term) = attr(&e, "term") {
                            primary_cat = term;
                        }
                    }
                    "link" => {
                        let title_attr = attr(&e, "title");
                        let rel = attr(&e, "rel");
                        let href = attr(&e, "href");
                        if title_attr.as_deref() == Some("pdf") {
                            pdf_url = href;
                        } else if rel.as_deref() == Some("alternate") {
                            abs_url = href;
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(e)) => {
                if !in_entry {
                    continue;
                }
                let text = e.unescape().unwrap_or_default().to_string();
                match current_tag.as_str() {
                    "id" => id.push_str(&text),
                    "title" => title.push_str(&normalize(&text)),
                    "summary" => summary.push_str(&normalize(&text)),
                    "published" => published.push_str(&text),
                    "updated" => updated.push_str(&text),
                    "name" if in_author => authors.push(text),
                    "doi" => doi = Some(text.trim().to_string()),
                    "journal_ref" => journal_ref = Some(normalize(&text)),
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "author" => in_author = false,
                    "entry" => {
                        in_entry = false;
                        let arxiv_id = id
                            .rsplit("/abs/")
                            .next()
                            .unwrap_or(&id)
                            .to_string();
                        let pdf = pdf_url.clone().unwrap_or_else(|| {
                            format!("https://arxiv.org/pdf/{arxiv_id}")
                        });
                        let abs = abs_url.clone().unwrap_or_else(|| {
                            format!("https://arxiv.org/abs/{arxiv_id}")
                        });
                        let pc = if primary_cat.is_empty() {
                            categories.first().cloned().unwrap_or_default()
                        } else {
                            primary_cat.clone()
                        };
                        papers.push(Paper {
                            arxiv_id,
                            title: title.trim().to_string(),
                            summary: summary.trim().to_string(),
                            authors: authors.clone(),
                            categories: categories.clone(),
                            primary_category: pc,
                            published: normalize_date(&published),
                            updated: normalize_date(&updated),
                            pdf_url: pdf,
                            abs_url: abs,
                            doi: doi.clone(),
                            journal_ref: journal_ref.clone(),
                            local_pdf_path: None,
                            note: String::new(),
                            reading_status: "unread".to_string(),
                            last_opened: None,
                            trashed: false,
                        });
                    }
                    _ => {}
                }
                current_tag.clear();
            }
            _ => {}
        }
        buf.clear();
    }
    Ok(papers)
}

fn local_name(qname: &[u8]) -> String {
    let s = String::from_utf8_lossy(qname);
    match s.rsplit(':').next() {
        Some(n) => n.to_string(),
        None => s.to_string(),
    }
}

fn attr(e: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        if local_name(a.key.as_ref()) == key {
            Some(String::from_utf8_lossy(&a.value).to_string())
        } else {
            None
        }
    })
}

fn normalize(s: &str) -> String {
    s.replace('\n', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_date(s: &str) -> String {
    match s.parse::<DateTime<Utc>>() {
        Ok(d) => d.to_rfc3339(),
        Err(_) => s.to_string(),
    }
}
