//! Keeps the layers' arrows pointing inwards (Clean Architecture): `domain` knows nothing else, `application` only
//! `domain`, `infra` and `ui` don't know each other. Test modules may reach further, to try things for real.

use std::fs;
use std::path::Path;

/// Every `.rs` file under `dir`, with its `#[cfg(test)] mod tests` part left out.
fn sources(dir: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(dir)];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path).expect("the folder exists") {
            let path = entry.expect("readable").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = fs::read_to_string(&path).expect("readable");
                let code = match text.find("#[cfg(test)]\nmod tests") {
                    Some(i) => text[..i].to_string(),
                    None => text,
                };
                out.push((path.display().to_string(), code));
            }
        }
    }
    assert!(!out.is_empty(), "no sources in {dir}");
    out
}

fn forbid(dir: &str, words: &[&str]) {
    for (file, code) in sources(dir) {
        for line in code.lines().filter(|l| !l.trim_start().starts_with("//")) {
            for word in words {
                assert!(!line.contains(word), "{file} uses {word}: {line}");
            }
        }
    }
}

const UI: &[&str] = &["crate::ui", "crate::m3", "crate::theme", "crate::model3d", "eframe::", "egui::", "jni::"];
const INFRA: &[&str] = &["crate::infra", "crate::update", "crate::qr", "serde_json", "https://"];

#[test]
fn the_domain_stands_alone() {
    forbid("domain", &["crate::application", "crate::engine", "crate::nav", "crate::locale", "crate::demo"]);
    forbid("domain", UI);
    forbid("domain", INFRA);
}

#[test]
fn the_application_only_knows_the_domain_and_its_ports() {
    forbid("application", UI);
    forbid("application", INFRA);
}

#[test]
fn adapters_and_screens_dont_know_each_other() {
    forbid("infra", UI);
    forbid("ui", INFRA);
}
