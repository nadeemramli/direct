//! Static, read-only human index for a retained Linear source bundle.
//!
//! All source text is HTML-escaped. The page carries a Content-Security-Policy
//! of `default-src 'none'`, contains no script, loads nothing remote, and only
//! links to files that were verified into the bundle beside it.

use super::{
    package::VerifiedPackage,
    workspace::{Class, Entry, BUNDLE_DIR},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};

pub(super) fn render(package: &VerifiedPackage, entries: &[Entry], report: &Value) -> String {
    let mut html = String::new();
    html.push_str(concat!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n",
        "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\">\n",
        "<meta name=\"referrer\" content=\"no-referrer\">\n",
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
        "<title>Linear migration source index</title>\n<style>\n",
        ":root{color-scheme:light dark;--fg:#1d1f23;--bg:#fff;--muted:#5b6270;--line:#d8dce3;--warn:#8a3b00;--warnbg:#fff3e6}\n",
        "@media (prefers-color-scheme:dark){:root{--fg:#e6e8ec;--bg:#15171a;--muted:#9aa2b1;--line:#2d3138;--warn:#ffb070;--warnbg:#2a1d10}}\n",
        "body{font:14px/1.45 system-ui,sans-serif;color:var(--fg);background:var(--bg);margin:0 auto;max-width:1400px;padding:16px}\n",
        "table{border-collapse:collapse;width:100%;margin:8px 0 24px}th,td{border:1px solid var(--line);padding:4px 6px;text-align:left;vertical-align:top}\n",
        "th{background:color-mix(in srgb,var(--line) 40%,transparent)}td{overflow-wrap:anywhere}code,pre{font:12px/1.4 ui-monospace,monospace;white-space:pre-wrap;overflow-wrap:anywhere}\n",
        ".warn{border:1px solid var(--warn);background:var(--warnbg);padding:8px 12px}.muted{color:var(--muted)}\n",
        ".native{color:#1a7f37}.transformed{color:#9a6700}.preserved{color:#0969da}.unresolved{color:#cf222e;font-weight:600}\n",
        "</style>\n</head>\n<body>\n",
        "<h1>Linear migration rehearsal — source index</h1>\n",
    ));
    let source = &report["source"];
    let _ = writeln!(
        html,
        "<p class=\"muted\">Captured {} · manifest sha256 <code>{}</code> · Direct workspace <code>{}</code></p>",
        esc(source["captured_at"].as_str().unwrap_or("")),
        esc(source["manifest_sha256"].as_str().unwrap_or("")),
        esc(report["archive"]["workspace_id"].as_str().unwrap_or("")),
    );
    let _ = writeln!(
        html,
        "<div class=\"warn\"><p><strong>Cutover readiness: BLOCKED.</strong> This is an offline rehearsal. {}</p><p>This page is a read-only index. Links open files in the retained <code>{BUNDLE_DIR}/</code> directory beside it; source URLs are shown as text and are never fetched.</p></div>",
        esc(report["native_access"]["statement"].as_str().unwrap_or("")),
    );

    html.push_str("<h2>Blockers</h2>\n<ul>\n");
    for blocker in report["cutover_readiness"]["blockers"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let counts = blocker
            .get("counts")
            .map(|counts| format!(" <code>{}</code>", esc(&counts.to_string())))
            .or_else(|| {
                blocker
                    .get("count")
                    .map(|count| format!(" ({})", esc(&count.to_string())))
            })
            .unwrap_or_default();
        let _ = writeln!(
            html,
            "<li><code>{}</code> — {}{}</li>",
            esc(blocker["code"].as_str().unwrap_or("")),
            esc(blocker["detail"].as_str().unwrap_or("")),
            counts
        );
    }
    html.push_str("</ul>\n");

    let classes = [
        Class::Native,
        Class::Transformed,
        Class::Preserved,
        Class::Unresolved,
    ];
    let mut by_kind: BTreeMap<&str, Vec<&Entry>> = BTreeMap::new();
    for entry in entries {
        by_kind.entry(entry.kind.as_str()).or_default().push(entry);
    }
    html.push_str("<h2>Classification summary</h2>\n<table>\n<tr><th>Kind</th>");
    for class in classes {
        let _ = write!(html, "<th>{}</th>", class.as_str());
    }
    html.push_str("</tr>\n");
    for (kind, items) in &by_kind {
        let _ = write!(html, "<tr><td><a href=\"#{0}\">{0}</a></td>", esc(kind));
        for class in classes {
            let count = items
                .iter()
                .filter(|entry| entry.classification == class)
                .count();
            let _ = write!(html, "<td>{count}</td>");
        }
        html.push_str("</tr>\n");
    }
    html.push_str("</table>\n");

    let previous = report["previous_identifiers"].as_array();
    if previous.is_some_and(|items| !items.is_empty()) {
        html.push_str(
            "<h2>Previous identifiers</h2>\n<table>\n<tr><th>Previous Linear identifier</th><th>Current Direct key</th></tr>\n",
        );
        for item in previous.into_iter().flatten() {
            let _ = writeln!(
                html,
                "<tr><td>{}</td><td>{}</td></tr>",
                esc(item["previous"].as_str().unwrap_or("")),
                esc(item["direct_key"].as_str().unwrap_or("")),
            );
        }
        html.push_str("</table>\n");
    }

    let linkable: BTreeSet<&str> = package.files.keys().map(String::as_str).collect();
    html.push_str("<h2>Records</h2>\n");
    for (kind, items) in &by_kind {
        let _ = writeln!(
            html,
            "<h3 id=\"{0}\">{0} ({1})</h3>\n<table>\n<tr><th>Source</th><th>Source id</th><th>Classification</th><th>Direct</th><th>Source pointer</th><th>Reasons</th><th>Preserved-only fields</th></tr>",
            esc(kind),
            items.len()
        );
        for entry in items {
            let file = if linkable.contains(entry.file.as_str()) {
                format!(
                    "<a href=\"{BUNDLE_DIR}/{}\">{}</a>",
                    href(&entry.file),
                    esc(&entry.file)
                )
            } else {
                esc(&entry.file)
            };
            let direct = match entry.direct.get("bundle_path").and_then(Value::as_str) {
                Some(path)
                    if path
                        .strip_prefix(&format!("{BUNDLE_DIR}/"))
                        .is_some_and(|relative| linkable.contains(relative)) =>
                {
                    format!("<a href=\"{}\">{}</a>", href(path), esc(path))
                }
                _ if entry.direct.is_null() => String::new(),
                _ => format!("<code>{}</code>", esc(&entry.direct.to_string())),
            };
            let _ = writeln!(
                html,
                "<tr><td>{}</td><td><code>{}</code></td><td class=\"{2}\">{2}</td><td>{3}</td><td>{4} <code>{5}</code></td><td>{6}</td><td>{7}</td></tr>",
                match &entry.source_title {
                    Some(title) => format!(
                        "{} — {}",
                        esc(entry.source_label.as_deref().unwrap_or("")),
                        esc(title)
                    ),
                    None => esc(entry.source_label.as_deref().unwrap_or("")),
                },
                esc(entry.source_id.as_deref().unwrap_or("")),
                entry.classification.as_str(),
                direct,
                file,
                esc(&entry.pointer),
                entry
                    .reasons
                    .iter()
                    .map(|reason| esc(reason))
                    .collect::<Vec<_>>()
                    .join("<br>"),
                esc(&entry.preserved_fields.join(", ")),
            );
        }
        html.push_str("</table>\n");
    }

    let documents = package.records("documents.json");
    if !documents.is_empty() {
        html.push_str("<h2>Document text (escaped, not native Direct content)</h2>\n");
        for (index, document) in documents.iter().enumerate() {
            let _ = writeln!(
                html,
                "<details><summary>{} <code>data/documents.json /records/{index}</code></summary><pre>{}</pre></details>",
                esc(document.get("title").and_then(Value::as_str).unwrap_or("(untitled)")),
                esc(document.get("content").and_then(Value::as_str).unwrap_or("")),
            );
        }
    }
    html.push_str("</body>\n</html>\n");
    html
}

fn esc(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            other => output.push(other),
        }
    }
    output
}

/// Percent-encode a verified relative bundle path for an href attribute.
fn href(path: &str) -> String {
    let mut output = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            output.push(byte as char);
        } else {
            let _ = write!(output, "%{byte:02X}");
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_and_links_are_inert() {
        assert_eq!(
            esc("<script>alert('x')</script>&\""),
            "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;&amp;&quot;"
        );
        assert_eq!(
            href("attachments/a b\"<x>.png"),
            "attachments/a%20b%22%3Cx%3E.png"
        );
    }
}
