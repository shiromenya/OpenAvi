//! INI 風テキストの低レベルパーサー（意味解釈はしない）。

#[derive(Debug, Clone)]
pub struct RawSection {
    pub name: String,
    /// セクション見出しの行番号（1 始まり）
    pub line: usize,
    pub entries: Vec<(String, String)>,
}

impl RawSection {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// セクション列に分解します。セクション外の行は `warnings` に記録して無視します。
pub fn parse(text: &str, warnings: &mut Vec<String>) -> Vec<RawSection> {
    let mut sections: Vec<RawSection> = Vec::new();
    for (i, raw_line) in text.lines().enumerate() {
        let line_no = i + 1;
        let line = raw_line.trim_end_matches(['\r', '\0']);
        if line.trim().is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push(RawSection { name: name.trim().to_string(), line: line_no, entries: Vec::new() });
            continue;
        }
        let Some(cur) = sections.last_mut() else {
            warnings.push(format!("{line_no} 行目: セクション外の行を無視しました"));
            continue;
        };
        match line.split_once('=') {
            Some((k, v)) => cur.entries.push((k.to_string(), v.to_string())),
            None => warnings.push(format!("{line_no} 行目: `key=value` 形式ではない行を無視しました")),
        }
    }
    sections
}
