//! Reads the word lists in `words.txt`.

use std::{collections::HashMap, sync::OnceLock};

/// One language's sounds: the parts its words are built from, and its own ways of
/// naming each kind of place.
#[derive(Default, Debug)]
pub(crate) struct Tongue {
    pub name: String,
    pub first: Vec<String>,
    pub second: Vec<String>,
    /// Given names, for places named after someone.
    pub given: Vec<String>,
    /// Patterns by kind of place, used alongside the kind's own.
    pub patterns: HashMap<String, Vec<String>>,
}

/// The words for one kind of place.
#[derive(Default, Debug)]
pub(crate) struct KindWords {
    pub pattern: Vec<String>,
    pub kind: Vec<String>,
    pub role: Vec<String>,
    pub adj: Vec<String>,
}

#[derive(Default, Debug)]
pub(crate) struct Words {
    /// In the order the file gives them, which is how regions refer to them.
    pub tongues: Vec<Tongue>,
    pub features: HashMap<String, Vec<String>>,
    pub kinds: HashMap<String, KindWords>,
}

impl Words {
    pub fn tongue_named(&self, name: &str) -> usize {
        self.tongues.iter().position(|t| t.name == name).unwrap_or(0)
    }
}

/// The built-in word lists, read once.
pub(crate) fn words() -> &'static Words {
    static WORDS: OnceLock<Words> = OnceLock::new();
    WORDS.get_or_init(|| parse(include_str!("words.txt")))
}

/// Values on one line: split on `|` if there is one, else on spaces.
fn values(text: &str) -> Vec<String> {
    let parts: Vec<&str> = if text.contains('|') { text.split('|').collect() } else { text.split_whitespace().collect() };
    parts.into_iter().map(str::trim).filter(|v| !v.is_empty()).map(String::from).collect()
}

pub(crate) fn parse(text: &str) -> Words {
    let mut w = Words::default();
    let (mut group, mut name) = (String::new(), String::new());
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(head) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let mut parts = head.split_whitespace();
            group = parts.next().unwrap_or("").to_string();
            name = parts.next().unwrap_or("").to_string();
            if group == "tongue" {
                w.tongues.push(Tongue { name: name.clone(), ..Default::default() });
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, list) = (key.trim(), values(value));
        match group.as_str() {
            "tongue" => {
                let Some(t) = w.tongues.last_mut() else { continue };
                match key {
                    "first" => t.first = list,
                    "second" => t.second = list,
                    "given" => t.given = list,
                    other => {
                        t.patterns.insert(other.to_string(), list);
                    }
                }
            }
            "feature" => {
                w.features.insert(key.to_string(), list);
            }
            "kind" => {
                let k = w.kinds.entry(name.clone()).or_default();
                match key {
                    "pattern" => k.pattern = list,
                    "type" => k.kind = list,
                    "role" => k.role = list,
                    "adj" => k.adj = list,
                    _ => {}
                }
            }
            _ => {}
        }
    }
    w
}
