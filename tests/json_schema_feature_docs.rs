//! Every per-language table row, table column and section the JSON Schema
//! specs give Java, they give C# too, so the C# documentation cannot silently
//! lag the code. Java is the reference because it is the target the C# backend
//! is patterned after (PRINCIPLES §C#).

use std::fs;
use std::path::{Path, PathBuf};

fn spec_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("specs/json-schema");
    let mut files: Vec<PathBuf> = fs::read_dir(root.join("features"))
        .expect("read features directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    files.push(root.join("nullability.md"));
    files.push(root.join("services.md"));
    files.sort();
    files
}

/// The trimmed cells of a Markdown table row, with bold markers stripped.
fn cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().trim_matches('*').trim().to_string())
        .collect()
}

fn names_language(cell: &str, language: &str) -> bool {
    cell == language
}

#[test]
fn json_schema_specs_document_csharp_wherever_they_document_java() {
    let mut gaps = Vec::new();
    for path in spec_files() {
        let text = fs::read_to_string(&path).expect("read spec");
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let lines: Vec<&str> = text.lines().collect();

        // Tables: a Java column needs a C# column; a Java row needs a C# row.
        let mut index = 0;
        while index < lines.len() {
            if !lines[index].starts_with('|') {
                index += 1;
                continue;
            }
            let start = index;
            while index < lines.len() && lines[index].starts_with('|') {
                index += 1;
            }
            let table = &lines[start..index];
            let header = cells(table[0]);
            if header.iter().any(|cell| names_language(cell, "Java"))
                && !header.iter().any(|cell| names_language(cell, "C#"))
            {
                gaps.push(format!(
                    "{name}:{}: table has a Java column but no C# column",
                    start + 1
                ));
            }
            let first_cells: Vec<String> = table
                .iter()
                .skip(2)
                .map(|row| cells(row).into_iter().next().unwrap_or_default())
                .collect();
            if first_cells.iter().any(|cell| names_language(cell, "Java"))
                && !first_cells.iter().any(|cell| names_language(cell, "C#"))
            {
                gaps.push(format!(
                    "{name}:{}: table has a Java row but no C# row",
                    start + 1
                ));
            }
        }

        // Sections: a `### Java…` heading needs a `### C#…` heading at the
        // same level after it.
        for (line_index, line) in lines.iter().enumerate() {
            let level = line.chars().take_while(|c| *c == '#').count();
            if level == 0 || !line[level..].trim_start().starts_with("Java") {
                continue;
            }
            let prefix = format!("{} C#", "#".repeat(level));
            let next_csharp = lines[line_index + 1..]
                .iter()
                .take_while(|candidate| {
                    let candidate_level = candidate.chars().take_while(|c| *c == '#').count();
                    candidate_level == 0 || candidate_level >= level
                })
                .any(|candidate| candidate.starts_with(&prefix));
            if !next_csharp {
                gaps.push(format!(
                    "{name}:{}: `{line}` has no `{prefix}` section beside it",
                    line_index + 1
                ));
            }
        }
    }
    assert!(
        gaps.is_empty(),
        "the JSON Schema specs document Java without C#:\n  {}",
        gaps.join("\n  ")
    );
}
