use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashSet;
use std::future::Future;

static IMG_REF: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?i)\(\s*([^()\s"]+\.(?:jpe?g|png|gif|tiff?|svg|webp))(?:\s+"[^"]*")?\s*\)"#)
        .expect("valid image-ref regex")
});

/// Image filenames referenced in the exhibit markdown — in order, deduped.
/// Mirrors images.py::image_filenames.
pub fn image_filenames(markdown: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for cap in IMG_REF.captures_iter(markdown) {
        let fn_ = cap[1].trim().to_string();
        if !fn_.is_empty() && seen.insert(fn_.clone()) {
            out.push(fn_);
        }
    }
    out
}

/// Strip image refs (and their trailing labels) and markdown image syntax,
/// leaving candidate readable text. Mirrors the relevant part of api.clean_excerpt.
fn clean_excerpt(markdown: &str) -> String {
    // remove ![alt](...) image syntax, then bare (foo.jpg) refs
    let no_md_img = Regex::new(r"!\[[^\]]*\]\([^)]*\)").unwrap().replace_all(markdown, " ");
    let no_refs = IMG_REF.replace_all(&no_md_img, " ");
    // drop residual markdown punctuation/whitespace
    let stripped: String = no_refs
        .chars()
        .filter(|c| !matches!(c, '!' | '[' | ']' | '(' | ')' | '#' | '*' | '_' | '>' | '`' | '-'))
        .collect();
    stripped.trim().to_string()
}

/// True when the body is essentially just image refs: images present AND no
/// readable text survives clean_excerpt. Mirrors images.py::is_image_only.
pub fn is_image_only(markdown: &str) -> bool {
    if image_filenames(markdown).is_empty() {
        return false;
    }
    clean_excerpt(markdown).is_empty()
}

pub fn dataset_image_path(accession: &str, filename: &str) -> String {
    format!("images/{accession}/{filename}")
}

pub fn public_url(repo: &str, path_in_repo: &str) -> String {
    format!("https://huggingface.co/datasets/{repo}/resolve/main/{path_in_repo}")
}

/// Orchestrate capture with an injected async uploader. Filters graphics to
/// `only`, uploads as one commit, returns public URLs in order. On uploader
/// error returns []. Mirrors images.py::capture_images.
pub async fn capture_images_with<U, Fut>(
    accession: &str,
    graphics: Vec<(String, Vec<u8>)>,
    only: Option<&HashSet<String>>,
    repo: &str,
    uploader: U,
) -> Vec<String>
where
    U: FnOnce(Vec<(Vec<u8>, String)>) -> Fut,
    Fut: Future<Output = anyhow::Result<()>>,
{
    let selected: Vec<(String, Vec<u8>)> = graphics
        .into_iter()
        .filter(|(f, _)| only.map_or(true, |o| o.contains(f)))
        .collect();
    if selected.is_empty() {
        return Vec::new();
    }
    let mut uploads = Vec::new();
    let mut urls = Vec::new();
    for (filename, data) in selected {
        let path = dataset_image_path(accession, &filename);
        urls.push(public_url(repo, &path));
        uploads.push((data, path));
    }
    match uploader(uploads).await {
        Ok(()) => urls,
        Err(e) => {
            tracing::warn!("image upload failed for {accession}: {e}");
            Vec::new()
        }
    }
}

/// Upload blobs to an HF dataset in ONE commit via the Hub commit API (base64
/// inline). Mirrors images.py::upload_images. `uploads` is (bytes, path_in_repo).
pub async fn hf_upload(
    client: &reqwest::Client,
    repo: &str,
    token: &str,
    uploads: Vec<(Vec<u8>, String)>,
) -> anyhow::Result<()> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut ndjson = String::new();
    let header = serde_json::json!({
        "key": "header",
        "value": { "summary": format!("add {} exhibit image(s)", uploads.len()) }
    });
    ndjson.push_str(&serde_json::to_string(&header)?);
    ndjson.push('\n');
    for (data, path) in &uploads {
        let line = serde_json::json!({
            "key": "file",
            "value": { "path": path, "encoding": "base64", "content": b64.encode(data) }
        });
        ndjson.push_str(&serde_json::to_string(&line)?);
        ndjson.push('\n');
    }
    let url = format!("https://huggingface.co/api/datasets/{repo}/commit/main");
    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/x-ndjson")
        .body(ndjson)
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("HF commit failed {status}: {text}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_image_filenames_in_order_deduped() {
        let md = "text (ex10-3_001.jpg) more (exhibit101.png \"slide1\") (ex10-3_001.jpg)";
        assert_eq!(image_filenames(md), vec!["ex10-3_001.jpg", "exhibit101.png"]);
    }

    #[test]
    fn no_images_returns_empty() {
        assert!(image_filenames("plain text, no refs").is_empty());
        assert!(image_filenames("").is_empty());
    }

    #[test]
    fn image_only_detection() {
        // body that is only image refs → image-only
        assert!(is_image_only("![](a.jpg)\n\n(b.png)"));
        // body with real prose → not image-only
        assert!(!is_image_only("This Agreement is made as of (logo.png)."));
        // no images at all → not image-only
        assert!(!is_image_only("just words"));
    }

    #[test]
    fn dataset_path_and_url() {
        assert_eq!(dataset_image_path("0001-25-000001", "a.jpg"), "images/0001-25-000001/a.jpg");
        assert_eq!(
            public_url("arthrod/sec-ex10-exhibits", "images/x/a.jpg"),
            "https://huggingface.co/datasets/arthrod/sec-ex10-exhibits/resolve/main/images/x/a.jpg"
        );
    }

    #[tokio::test]
    async fn capture_filters_to_only_and_builds_urls() {
        let graphics = vec![
            ("ex10-3_001.jpg".to_string(), vec![1u8, 2, 3]),
            ("other.jpg".to_string(), vec![9u8]),
        ];
        let only: std::collections::HashSet<String> =
            ["ex10-3_001.jpg".to_string()].into_iter().collect();

        let uploaded = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let up = uploaded.clone();
        let uploader = move |paths: Vec<(Vec<u8>, String)>| {
            let up = up.clone();
            async move {
                for (_, p) in &paths { up.lock().unwrap().push(p.clone()); }
                Ok::<(), anyhow::Error>(())
            }
        };

        let urls = capture_images_with(
            "0001-25-000001", graphics, Some(&only),
            "arthrod/sec-ex10-exhibits", uploader,
        ).await;

        assert_eq!(urls, vec![
            "https://huggingface.co/datasets/arthrod/sec-ex10-exhibits/resolve/main/images/0001-25-000001/ex10-3_001.jpg"
        ]);
        assert_eq!(*uploaded.lock().unwrap(), vec!["images/0001-25-000001/ex10-3_001.jpg"]);
    }

    #[tokio::test]
    async fn capture_returns_empty_when_no_matches() {
        let graphics = vec![("nope.jpg".to_string(), vec![1u8])];
        let only: std::collections::HashSet<String> = ["x.jpg".to_string()].into_iter().collect();
        let uploader = |_p: Vec<(Vec<u8>, String)>| async { Ok::<(), anyhow::Error>(()) };
        let urls = capture_images_with("acc", graphics, Some(&only), "repo", uploader).await;
        assert!(urls.is_empty());
    }
}
