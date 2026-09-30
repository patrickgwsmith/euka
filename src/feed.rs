use crate::session::{DiffMode, Resource};
use quick_xml::events::Event;
use quick_xml::Reader;
use quick_xml::XmlVersion;
use std::collections::HashSet;

#[derive(Default)]
struct Entry {
    id: String,
    title: String,
    link: String,
}

pub fn is_atom_content_type(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/atom+xml"))
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

pub fn render(resource: &Resource) -> Result<String, String> {
    let mut reader = Reader::from_str(&resource.content);
    let mut path = Vec::new();
    let mut entries = Vec::new();
    let mut current: Option<Entry> = None;
    let mut saw_feed = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|error| format!("invalid Atom XML: {error}"))?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(tag) | Event::Empty(tag) => {
                let name = local_name(tag.name().as_ref()).to_owned();
                if path.is_empty() {
                    if name != "feed" {
                        return Err("XML root is not an Atom feed".into());
                    }
                    let namespace_name = tag
                        .name()
                        .as_ref()
                        .split_once(':')
                        .map(|(prefix, _)| format!("xmlns:{prefix}"))
                        .unwrap_or_else(|| "xmlns".to_owned());
                    let mut atom_namespace = false;
                    for attribute in tag.attributes() {
                        let attribute = attribute
                            .map_err(|error| format!("invalid Atom attribute: {error}"))?;
                        if attribute.key.as_ref() == namespace_name {
                            atom_namespace = attribute
                                .normalized_value(XmlVersion::Implicit1_0)
                                .map_err(|error| format!("invalid Atom attribute: {error}"))?
                                == "http://www.w3.org/2005/Atom";
                        }
                    }
                    if !atom_namespace {
                        return Err("XML root is not in the Atom namespace".into());
                    }
                    saw_feed = true;
                }
                if path.len() == 1 && name == "entry" {
                    current = Some(Entry::default());
                }
                if path.len() == 2 && path[1] == "entry" && name == "link" {
                    let mut href = None;
                    let mut rel = None;
                    for attribute in tag.attributes() {
                        let attribute = attribute
                            .map_err(|error| format!("invalid Atom attribute: {error}"))?;
                        let value = attribute
                            .normalized_value(XmlVersion::Implicit1_0)
                            .map_err(|error| format!("invalid Atom attribute: {error}"))?
                            .into_owned();
                        match local_name(attribute.key.as_ref()) {
                            "href" => href = Some(value),
                            "rel" => rel = Some(value),
                            _ => {}
                        }
                    }
                    if rel.as_deref().is_none_or(|value| value == "alternate") {
                        if let (Some(entry), Some(href)) = (&mut current, href) {
                            entry.link = href;
                        }
                    }
                }
                path.push(name);
                if empty {
                    path.pop();
                }
            }
            Event::End(_) => {
                if path.len() == 2 && path[1] == "entry" {
                    if let Some(entry) = current.take() {
                        if entry.id.trim().is_empty() || entry.title.trim().is_empty() {
                            return Err("Atom entry is missing an id or title".into());
                        }
                        let title = one_line(&entry.title);
                        let link = if entry.link.is_empty() {
                            &entry.id
                        } else {
                            &entry.link
                        };
                        entries.push(format!("{title} — {link}"));
                    }
                }
                path.pop();
            }
            Event::Text(text) if path.len() >= 3 && path[1] == "entry" => {
                if let Some(entry) = &mut current {
                    match path[2].as_str() {
                        "id" => entry.id.push_str(&text.xml10_content()),
                        "title" => entry.title.push_str(&text.xml10_content()),
                        _ => {}
                    }
                }
            }
            Event::CData(text) if path.len() >= 3 && path[1] == "entry" && path[2] == "title" => {
                if let Some(entry) = &mut current {
                    entry.title.push_str(&text.xml10_content());
                }
            }
            Event::GeneralRef(reference) if path.len() >= 3 && path[1] == "entry" => {
                let character = reference
                    .resolve_char_ref()
                    .map_err(|error| format!("invalid Atom entity: {error}"))?
                    .or_else(|| match reference.as_ref() {
                        "amp" => Some('&'),
                        "lt" => Some('<'),
                        "gt" => Some('>'),
                        "quot" => Some('"'),
                        "apos" => Some('\''),
                        _ => None,
                    })
                    .ok_or("unsupported Atom entity")?;
                if let Some(entry) = &mut current {
                    match path[2].as_str() {
                        "id" => entry.id.push(character),
                        "title" => entry.title.push(character),
                        _ => {}
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !saw_feed {
        return Err("response is not an Atom feed".into());
    }
    Ok(entries.join("\n"))
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn diff(previous: &str, current: &str, mode: DiffMode) -> String {
    let old: HashSet<_> = previous.lines().collect();
    let new: HashSet<_> = current.lines().collect();
    let mut lines = Vec::new();
    if mode == DiffMode::AddedAndRemoved {
        for line in previous.lines().filter(|line| !new.contains(line)) {
            lines.push(format!("-{line}"));
        }
    }
    for line in current.lines().filter(|line| !old.contains(line)) {
        lines.push(format!("+{line}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atom_entries_render_and_diff_without_reorder_noise() {
        let resource = Resource {
            url: "https://example.com/commits.atom".into(),
            content_type: "application/atom+xml; charset=utf-8".into(),
            content: r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"><entry><id>tag:example,1</id><title>Fix &amp; test</title><link href="https://example.com/1"/></entry><entry><id>tag:example,2</id><title>Other</title><link rel="alternate" href="https://example.com/2"/></entry></feed>"#.into(),
        };
        let rendered = render(&resource).unwrap();
        assert_eq!(
            rendered,
            "Fix & test — https://example.com/1\nOther — https://example.com/2"
        );
        assert!(is_atom_content_type(&resource.content_type));
        assert_eq!(
            diff("Other — https://example.com/2", &rendered, DiffMode::Added),
            "+Fix & test — https://example.com/1"
        );
        assert_eq!(
            diff(
                &rendered,
                "Other — https://example.com/2",
                DiffMode::AddedAndRemoved
            ),
            "-Fix & test — https://example.com/1"
        );
    }
}
