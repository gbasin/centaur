//! Live HTTP `AtriumClient` (egress-only, x-api-key). Talks to the internal
//! node-ingestion endpoints: capture (POST raw bytes), raw fetch (GET bytes),
//! and the gap-free change-feed (GET JSON). Blocking (ureq) — the daemon loop is
//! synchronous per session.

use crate::adopt::RemoteChange;
use crate::runtime::{AtriumClient, status_of};

pub struct HttpAtriumClient {
    base_url: String,
    api_key: String,
    session_id: String,
    agent: ureq::Agent,
}

impl HttpAtriumClient {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            session_id: session_id.into(),
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(30))
                .build(),
        }
    }

    fn url(&self, suffix: &str) -> String {
        format!(
            "{}/api/internal/sessions/{}{}",
            self.base_url, self.session_id, suffix
        )
    }
}

fn enc(s: &str) -> String {
    // minimal percent-encoding for a path query value
    s.bytes()
        .map(|b| match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

impl AtriumClient for HttpAtriumClient {
    fn post_capture(&mut self, path: &str, base_seq: u64, bytes: &[u8]) -> Result<u64, String> {
        let mut req = self
            .agent
            .post(&self.url(&format!("/artifacts/capture?path={}", enc(path))))
            .set("x-api-key", &self.api_key)
            .set("content-type", "application/octet-stream");
        if base_seq > 0 {
            req = req.set("x-artifact-base-seq", &base_seq.to_string());
        }
        let resp = req
            .send_bytes(bytes)
            .map_err(|e| format!("capture {path}: {e}"))?;
        let v: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
        v.get("seq")
            .and_then(|s| s.as_u64())
            .ok_or_else(|| "no seq in capture response".to_string())
    }

    fn post_capture_stream(
        &mut self,
        path: &str,
        base_seq: u64,
        reader: &mut dyn std::io::Read,
        size_hint: u64,
    ) -> Result<u64, String> {
        // Stream the body (chunked) — ureq reads `reader` to EOF without buffering it
        // whole, so an arbitrarily large file uploads in constant node memory. The
        // dedicated /capture-stream route streams the chunked body straight to S3;
        // x-artifact-size is an informational size hint.
        let mut req = self
            .agent
            .post(&self.url(&format!("/artifacts/capture-stream?path={}", enc(path))))
            .set("x-api-key", &self.api_key)
            .set("content-type", "application/octet-stream")
            .set("x-artifact-size", &size_hint.to_string());
        if base_seq > 0 {
            req = req.set("x-artifact-base-seq", &base_seq.to_string());
        }
        let resp = req
            .send(reader)
            .map_err(|e| format!("stream {path}: {e}"))?;
        let v: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
        v.get("seq")
            .and_then(|s| s.as_u64())
            .ok_or_else(|| "no seq in capture response".to_string())
    }

    fn post_delete(&mut self, path: &str, base_seq: u64) -> Result<u64, String> {
        let mut req = self
            .agent
            .post(&self.url(&format!("/artifacts/capture?path={}", enc(path))))
            .set("x-api-key", &self.api_key)
            .set("x-artifact-delete", "true");
        if base_seq > 0 {
            req = req.set("x-artifact-base-seq", &base_seq.to_string());
        }
        let resp = req
            .send_bytes(&[])
            .map_err(|e| format!("delete {path}: {e}"))?;
        let v: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
        v.get("seq")
            .and_then(|s| s.as_u64())
            .ok_or_else(|| "no seq in delete response".to_string())
    }

    fn fetch_bytes(&mut self, path: &str, seq: u64) -> Result<Vec<u8>, String> {
        let resp = self
            .agent
            .get(&self.url(&format!("/artifacts/raw?path={}&seq={}", enc(path), seq)))
            .set("x-api-key", &self.api_key)
            .call()
            .map_err(|e| format!("fetch {path}@{seq}: {e}"))?;
        let mut buf = Vec::new();
        std::io::copy(&mut resp.into_reader(), &mut buf).map_err(|e| e.to_string())?;
        Ok(buf)
    }

    fn put_harness_transcript(&mut self, harness: &str, bytes: &[u8]) -> Result<(), String> {
        self.agent
            .put(&self.url(&format!("/harness-transcript?harness={}", enc(harness))))
            .set("x-api-key", &self.api_key)
            .set("content-type", "application/jsonl")
            .send_bytes(bytes)
            .map(|_| ())
            .map_err(|e| format!("put harness transcript {harness}: {e}"))
    }

    fn poll_changes(
        &mut self,
        cursor: &str,
    ) -> Result<(Vec<(String, RemoteChange)>, String), String> {
        // The change-feed lives at the public session route (the node is allowed to
        // read it via the same api-key gate exposed on the internal mirror).
        let resp = self
            .agent
            .get(&self.url(&format!("/artifacts/changes?since={}", enc(cursor))))
            .set("x-api-key", &self.api_key)
            .call()
            .map_err(|e| format!("poll {cursor}: {e}"))?;
        let v: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
        let next = v
            .get("next_cursor")
            .and_then(|c| c.as_str())
            .unwrap_or(cursor)
            .to_string();
        let mut out = Vec::new();
        if let Some(rows) = v.get("rows").and_then(|r| r.as_array()) {
            for row in rows {
                let path = row
                    .get("path")
                    .and_then(|p| p.as_str())
                    .unwrap_or_default()
                    .to_string();
                let seq = row.get("seq").and_then(|s| s.as_u64()).unwrap_or(0);
                let sha = row
                    .get("sha")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string());
                let status = status_of(
                    row.get("status")
                        .and_then(|s| s.as_str())
                        .unwrap_or("normal"),
                );
                let group_id = row
                    .get("group_id")
                    .and_then(|g| g.as_str())
                    .map(|g| g.to_string());
                out.push((
                    path,
                    RemoteChange {
                        seq,
                        sha,
                        status,
                        group_id,
                    },
                ));
            }
        }
        Ok((out, next))
    }

    fn atrium_changes(&self, since: &str) -> Result<(Vec<String>, String), String> {
        let resp = self
            .agent
            .get(&self.url(&format!("/atrium/changes?since={}", enc(since))))
            .set("x-api-key", &self.api_key)
            .call()
            .map_err(|e| format!("atrium changes {since}: {e}"))?;
        let v: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
        let next = v
            .get("next_cursor")
            .and_then(|c| c.as_str())
            .unwrap_or(since)
            .to_string();
        let mut session_ids = Vec::new();
        if let Some(rows) = v.get("rows").and_then(|r| r.as_array()) {
            for row in rows {
                if let Some(session_id) = row.get("sessionId").and_then(|s| s.as_str()) {
                    session_ids.push(session_id.to_string());
                }
            }
        }
        Ok((session_ids, next))
    }

    fn atrium_doc(&self, target_id: &str, doc: &str) -> Result<Vec<u8>, String> {
        let resp = self
            .agent
            .get(&self.url(&format!("/atrium/sessions/{}/{}", enc(target_id), doc)))
            .set("x-api-key", &self.api_key)
            .call()
            .map_err(|e| format!("atrium doc {target_id}/{doc}: {e}"))?;
        let mut buf = Vec::new();
        std::io::copy(&mut resp.into_reader(), &mut buf).map_err(|e| e.to_string())?;
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_transcript_url_uses_internal_session_endpoint() {
        let client = HttpAtriumClient::new("http://atrium/", "key", "slack:C123:123.456");

        assert_eq!(
            client.url("/harness-transcript?harness=claude"),
            "http://atrium/api/internal/sessions/slack:C123:123.456/harness-transcript?harness=claude"
        );
    }
}
