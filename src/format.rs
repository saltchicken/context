use crate::fs::FsData;
use std::borrow::Cow;
use std::fmt::Write;

/// Escapes XML characters in a single pass.
/// Returns `Cow::Borrowed` (no allocation) if no escaping was needed.
fn escape_xml(input: &str) -> Cow<'_, str> {
    let mut last_end = 0;
    let mut escaped: Option<String> = None;

    for (i, c) in input.char_indices() {
        let escape = match c {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&apos;",
            _ => continue,
        };

        if escaped.is_none() {
            escaped = Some(String::with_capacity(input.len() + 16)); // Slight overallocation for escapes
        }

        let s = escaped.as_mut().unwrap();
        s.push_str(&input[last_end..i]);
        s.push_str(escape);
        last_end = i + c.len_utf8();
    }

    if let Some(mut s) = escaped {
        s.push_str(&input[last_end..]);
        Cow::Owned(s)
    } else {
        Cow::Borrowed(input)
    }
}

/// Helper to estimate the required buffer capacity to avoid constant re-allocations
fn estimate_capacity(
    instructions: Option<&str>,
    prompt: Option<&str>,
    fs: Option<&[FsData]>,
) -> usize {
    let mut cap = 0;
    if let Some(i) = instructions {
        cap += i.len() + 32; // Added capacity for wrapper tags or headers
    }
    if let Some(p) = prompt {
        cap += p.len() + 32; // Added capacity for wrapper tags or headers
    }
    if let Some(fs_list) = fs {
        for fs_item in fs_list {
            cap += fs_item.project_name.len() + 64; // Capacity for project tags
            cap += fs_item.tree.len() + 64;
            for f in &fs_item.files {
                cap += f.path.len()
                    + f.content.as_ref().map_or(0, |c| c.len())
                    + f.error.as_ref().map_or(0, |e| e.len())
                    + f.skipped.as_ref().map_or(0, |s| s.len())
                    + 128; // Wrapper tags overhead
            }
        }
    }
    cap
}

pub fn format_output(
    instructions: Option<&str>,
    prompt: Option<&str>,
    fs: Option<&[FsData]>,
) -> String {
    let capacity = estimate_capacity(instructions, prompt, fs);
    let mut out = String::with_capacity(capacity);

    if let Some(i) = instructions {
        out.push_str("<instructions>\n");
        out.push_str(i);
        out.push_str("\n</instructions>\n\n");
    }

    if let Some(p) = prompt {
        out.push_str("<prompt>\n");
        out.push_str(p);
        out.push_str("\n</prompt>\n\n");
    }

    if let Some(fs_list) = fs {
        for fs_item in fs_list {
            let _ = write!(out, "<project name=\"{}\">\n", escape_xml(&fs_item.project_name));

            out.push_str("<directory_structure>\n");
            out.push_str(&fs_item.tree);
            out.push_str("\n</directory_structure>\n");

            if !fs_item.files.is_empty() {
                out.push_str("<file_contents>\n");
                for f in &fs_item.files {
                    if let Some(err) = &f.error {
                        let _ = write!(
                            out,
                            "<file path=\"{}\" error=\"true\">\nError: {}\n</file>\n",
                            escape_xml(&f.path),
                            escape_xml(err)
                        );
                    } else if let Some(skip) = &f.skipped {
                        let _ = write!(
                            out,
                            "<file path=\"{}\" skipped=\"true\">\nSkipped: {}\n</file>\n",
                            escape_xml(&f.path),
                            escape_xml(skip)
                        );
                    } else if let Some(content) = &f.content {
                        let _ = write!(
                            out,
                            "<file path=\"{}\">\n",
                            escape_xml(&f.path)
                        );
                        out.push_str(content);
                        if !content.ends_with('\n') {
                            out.push('\n');
                        }
                        out.push_str("</file>\n");
                    }
                }
                out.push_str("</file_contents>\n");
            }

            out.push_str("</project>\n");
        }
    }

    out.trim_end().to_string()
}
