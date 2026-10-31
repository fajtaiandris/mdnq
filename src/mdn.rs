use std::{
    fs,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use scraper::{ElementRef, Html, Selector};
use url::Url;

use crate::search::SearchItem;

const BASE: &str = "https://developer.mozilla.org";
const INDEX_URL: &str = "https://developer.mozilla.org/en-US/search-index.json";

#[derive(Clone, Debug)]
pub struct Example {
    pub code: String,
    pub language: String,
    pub caption: String,
}

#[derive(Clone, Debug)]
pub enum Block {
    Heading(u8, String),
    Paragraph(String),
    Bullet(String),
    Quote(String),
    Code(String, String),
    Image(String),
    Table(Vec<Vec<String>>),
    Rule,
}

#[derive(Clone, Debug)]
pub struct Page {
    pub title: String,
    pub url: String,
    pub blocks: Vec<Block>,
    pub examples: Vec<Example>,
}

pub struct Mdn {
    client: Client,
}

impl Mdn {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .user_agent(concat!("mdnq/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(18))
            .build()?;
        Ok(Self { client })
    }

    fn get(&self, url: &str) -> Result<String> {
        Ok(self.client.get(url).send()?.error_for_status()?.text()?)
    }

    pub fn load_index(&self) -> Result<(Vec<SearchItem>, bool)> {
        let path = cache_path();
        if let Ok(meta) = fs::metadata(&path) {
            if meta
                .modified()
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age < Duration::from_secs(24 * 3600))
            {
                if let Ok(index) = read_index(&path) {
                    return Ok((index, false));
                }
            }
        }
        match self.get(INDEX_URL).and_then(|body| {
            let index: Vec<SearchItem> = serde_json::from_str(&body)?;
            if index.is_empty() {
                return Err(anyhow!("MDN returned an empty search index"));
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, body)?;
            Ok(index)
        }) {
            Ok(index) => Ok((index, false)),
            Err(error) => read_index(&path).map(|index| (index, true)).context(error),
        }
    }

    pub fn load_page(&self, url: &str, fallback_title: &str) -> Result<Page> {
        let url = normalize_url(url)?;
        let html = self.get(&url);
        if let Ok(ref body) = html {
            let page = parse_html(body, &url, fallback_title);
            if !page.blocks.is_empty() {
                return Ok(page);
            }
        }
        let source = source_url(&url)?;
        let markdown = self.get(&source).with_context(|| match html {
            Ok(_) => "MDN page had no readable content".to_string(),
            Err(error) => format!("MDN page could not load: {error}"),
        })?;
        Ok(parse_markdown(&markdown, &url, fallback_title))
    }
}

fn cache_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("mdnq/search-index.json")
}

fn read_index(path: &PathBuf) -> Result<Vec<SearchItem>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

pub fn normalize_url(raw: &str) -> Result<String> {
    let base = Url::parse(BASE)?;
    let parsed = base.join(raw)?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("developer.mozilla.org")
        || parsed.port().is_some()
        || !parsed.path().starts_with("/en-US/docs/")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(anyhow!("Choose an HTTPS en-US MDN documentation page"));
    }
    Ok(format!("{BASE}{}", parsed.path()))
}

fn source_url(url: &str) -> Result<String> {
    let parsed = Url::parse(url)?;
    let slug = parsed
        .path()
        .trim_start_matches("/en-US/docs/")
        .to_lowercase();
    Ok(format!(
        "https://raw.githubusercontent.com/mdn/content/main/files/en-us/{slug}/index.md"
    ))
}

fn parse_html(html: &str, url: &str, fallback_title: &str) -> Page {
    let document = Html::parse_document(html);
    let selector = Selector::parse("article, main").expect("valid selector");
    let root = document.select(&selector).next();
    let mut blocks = Vec::new();
    let mut examples = Vec::new();
    let mut caption = String::new();
    if let Some(root) = root {
        walk(root, &mut blocks, &mut examples, &mut caption);
    }
    let title = blocks
        .iter()
        .find_map(|block| match block {
            Block::Heading(1, value) => Some(value.clone()),
            _ => None,
        })
        .unwrap_or_else(|| fallback_title.to_string());
    Page {
        title,
        url: url.into(),
        blocks,
        examples,
    }
}

fn walk(
    element: ElementRef<'_>,
    blocks: &mut Vec<Block>,
    examples: &mut Vec<Example>,
    caption: &mut String,
) {
    for child in element.child_elements() {
        let tag = child.value().name();
        if matches!(
            tag,
            "nav" | "script" | "style" | "footer" | "header" | "button"
        ) {
            continue;
        }
        match tag {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let value = inline_text(child);
                if !value.is_empty() {
                    blocks.push(Block::Heading(tag[1..].parse().unwrap_or(2), value));
                }
                caption.clear();
            }
            "p" => {
                let value = inline_text(child);
                if !value.is_empty() {
                    *caption = last_sentence(&value);
                    blocks.push(Block::Paragraph(value));
                }
            }
            "pre" => {
                let code = child
                    .text()
                    .collect::<String>()
                    .trim_matches('\n')
                    .to_string();
                if !code.trim().is_empty() {
                    let language = code_language(child);
                    blocks.push(Block::Code(code.clone(), language.clone()));
                    examples.push(Example {
                        code,
                        language,
                        caption: caption.clone(),
                    });
                    caption.clear();
                }
            }
            "ul" | "ol" => {
                let ordered = tag == "ol";
                for (index, item) in child
                    .child_elements()
                    .filter(|e| e.value().name() == "li")
                    .enumerate()
                {
                    let value = inline_text_filtered(item, true);
                    if !value.is_empty() {
                        blocks.push(Block::Bullet(format!(
                            "{} {value}",
                            if ordered {
                                format!("{}.", index + 1)
                            } else {
                                "•".into()
                            }
                        )));
                    }
                    for nested in item
                        .child_elements()
                        .filter(|e| matches!(e.value().name(), "pre" | "ul" | "ol"))
                    {
                        walk_nested(nested, blocks, examples, caption);
                    }
                }
            }
            "blockquote" => {
                let value = inline_text(child);
                if !value.is_empty() {
                    blocks.push(Block::Quote(value));
                }
            }
            "dt" | "summary" => {
                let value = inline_text(child);
                if !value.is_empty() {
                    blocks.push(Block::Heading(4, value));
                }
            }
            "dd" | "figcaption" => {
                let value = inline_text(child);
                if !value.is_empty() {
                    blocks.push(Block::Paragraph(value));
                }
            }
            "table" => {
                let row_selector = Selector::parse("tr").expect("valid selector");
                let cell_selector = Selector::parse("th, td").expect("valid selector");
                let rows = child
                    .select(&row_selector)
                    .map(|row| {
                        row.select(&cell_selector)
                            .map(inline_text)
                            .collect::<Vec<_>>()
                    })
                    .filter(|row| !row.is_empty())
                    .collect::<Vec<_>>();
                if !rows.is_empty() {
                    blocks.push(Block::Table(rows));
                }
            }
            "img" => blocks.push(Block::Image(image_label(child))),
            "svg" => blocks.push(Block::Image("Inline SVG illustration".into())),
            "iframe" => blocks.push(Block::Image(format!(
                "Interactive content: {}",
                child.attr("title").unwrap_or("embedded example")
            ))),
            "video" | "audio" | "canvas" => blocks.push(Block::Image(format!(
                "{} content",
                match tag {
                    "video" => "Video",
                    "audio" => "Audio",
                    _ => "Canvas",
                }
            ))),
            "hr" => blocks.push(Block::Rule),
            _ => walk(child, blocks, examples, caption),
        }
    }
}

fn walk_nested(
    element: ElementRef<'_>,
    blocks: &mut Vec<Block>,
    examples: &mut Vec<Example>,
    caption: &mut String,
) {
    let wrapper = Html::parse_fragment(&element.html());
    walk(wrapper.root_element(), blocks, examples, caption);
}

fn inline_text(element: ElementRef<'_>) -> String {
    inline_text_filtered(element, false)
}

fn inline_text_filtered(element: ElementRef<'_>, skip_nested_blocks: bool) -> String {
    fn collect(element: ElementRef<'_>, out: &mut String, skip_nested_blocks: bool) {
        for child in element.children() {
            if let Some(text) = child.value().as_text() {
                out.push_str(&text.text);
            }
            if let Some(el) = ElementRef::wrap(child) {
                match el.value().name() {
                    "img" => {
                        out.push(' ');
                        out.push_str(&format!("[Image: {}]", image_label(el)));
                        out.push(' ');
                    }
                    "svg" => out.push_str(" [SVG illustration] "),
                    "br" => out.push(' '),
                    "script" | "style" => {}
                    "pre" | "ul" | "ol" if skip_nested_blocks => {}
                    _ => collect(el, out, skip_nested_blocks),
                }
            }
        }
    }
    let mut value = String::new();
    collect(element, &mut value, skip_nested_blocks);
    normalize_spaces(&value)
}

fn image_label(element: ElementRef<'_>) -> String {
    let alt = element.attr("alt").unwrap_or("").trim();
    let src = element.attr("src").unwrap_or("");
    match (alt.is_empty(), src.is_empty()) {
        (false, false) => format!("{alt} — {src}"),
        (false, true) => alt.to_string(),
        (true, false) => src.to_string(),
        (true, true) => "image".to_string(),
    }
}

fn code_language(element: ElementRef<'_>) -> String {
    let code = Selector::parse("code").expect("valid selector");
    let class = element
        .select(&code)
        .next()
        .and_then(|el| el.attr("class"))
        .or_else(|| element.attr("class"))
        .unwrap_or("");
    let mut parts = class.split_whitespace();
    while let Some(part) = parts.next() {
        if part == "brush:" {
            return parts.next().unwrap_or("").to_string();
        }
        if let Some(value) = part
            .strip_prefix("language-")
            .or_else(|| part.strip_prefix("brush:"))
        {
            return value.trim().to_string();
        }
    }
    String::new()
}

fn last_sentence(text: &str) -> String {
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if matches!(character, '.' | '!' | '?')
            && text[index + character.len_utf8()..].starts_with(' ')
        {
            start = index + character.len_utf8() + 1;
        }
    }
    text[start..].trim().to_string()
}

fn normalize_spaces(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_markdown(source: &str, url: &str, fallback_title: &str) -> Page {
    let mut blocks = Vec::new();
    let mut examples = Vec::new();
    let mut paragraph = Vec::new();
    let mut fence: Option<(char, usize, String)> = None;
    let mut code = Vec::new();
    let mut caption = String::new();
    let mut frontmatter = false;
    for (index, line) in source.lines().enumerate() {
        if index == 0 && line == "---" {
            frontmatter = true;
            continue;
        }
        if frontmatter {
            if line == "---" {
                frontmatter = false;
            }
            continue;
        }
        let trimmed = line.trim();
        if let Some((marker, length, language)) = &fence {
            if trimmed.chars().take_while(|c| c == marker).count() >= *length {
                let body = code.join("\n");
                if !body.trim().is_empty() {
                    blocks.push(Block::Code(body.clone(), language.clone()));
                    examples.push(Example {
                        code: body,
                        language: language.clone(),
                        caption: caption.clone(),
                    });
                }
                code.clear();
                fence = None;
                caption.clear();
            } else {
                code.push(line.to_string());
            }
            continue;
        }
        let marker = trimmed.chars().next().unwrap_or(' ');
        if (marker == '`' || marker == '~')
            && trimmed.chars().take_while(|c| *c == marker).count() >= 3
        {
            flush_paragraph(&mut paragraph, &mut blocks, &mut caption);
            let count = trimmed.chars().take_while(|c| *c == marker).count();
            fence = Some((marker, count, trimmed[count..].trim().to_string()));
        } else if trimmed.starts_with('#') {
            flush_paragraph(&mut paragraph, &mut blocks, &mut caption);
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            blocks.push(Block::Heading(
                level.min(6) as u8,
                clean_markdown(trimmed[level..].trim()),
            ));
        } else if trimmed.starts_with("![") {
            flush_paragraph(&mut paragraph, &mut blocks, &mut caption);
            blocks.push(Block::Image(trimmed.to_string()));
        } else if trimmed.starts_with("{{EmbedLiveSample(")
            || trimmed.starts_with("{{LiveSampleLink(")
        {
            flush_paragraph(&mut paragraph, &mut blocks, &mut caption);
            blocks.push(Block::Image(format!("Interactive example: {trimmed}")));
        } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            flush_paragraph(&mut paragraph, &mut blocks, &mut caption);
            blocks.push(Block::Bullet(format!(
                "• {}",
                clean_markdown(&trimmed[2..])
            )));
        } else if trimmed.starts_with('>') {
            flush_paragraph(&mut paragraph, &mut blocks, &mut caption);
            blocks.push(Block::Quote(clean_markdown(
                trimmed.trim_start_matches('>').trim(),
            )));
        } else if trimmed.is_empty() {
            flush_paragraph(&mut paragraph, &mut blocks, &mut caption);
        } else {
            paragraph.push(trimmed.to_string());
        }
    }
    flush_paragraph(&mut paragraph, &mut blocks, &mut caption);
    let title = blocks
        .iter()
        .find_map(|block| match block {
            Block::Heading(1, value) => Some(value.clone()),
            _ => None,
        })
        .unwrap_or_else(|| fallback_title.to_string());
    Page {
        title,
        url: url.to_string(),
        blocks,
        examples,
    }
}

fn flush_paragraph(paragraph: &mut Vec<String>, blocks: &mut Vec<Block>, caption: &mut String) {
    if !paragraph.is_empty() {
        let text = clean_markdown(&paragraph.join(" "));
        *caption = last_sentence(&text);
        blocks.push(Block::Paragraph(text));
        paragraph.clear();
    }
}

fn clean_markdown(value: &str) -> String {
    value
        .replace("**", "")
        .replace('`', "")
        .replace("{{", "")
        .replace("}}", "")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_preserves_reading_order_and_media_replacements() {
        let html = r#"<main><article><h1>Media</h1><p>First sentence. Try this:</p><pre><code class="language-css">@media (color) {}</code></pre><p>Here is <img alt="A colorful diagram" src="/diagram.svg">.</p><ul><li>One</li><li>Two</li></ul></article></main>"#;
        let page = parse_html(
            html,
            "https://developer.mozilla.org/en-US/docs/Web/CSS",
            "Fallback",
        );
        assert_eq!(page.title, "Media");
        assert_eq!(page.examples[0].caption, "Try this:");
        assert_eq!(page.examples[0].language, "css");
        assert!(page
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Paragraph(s) if s.contains("[Image: A colorful diagram"))));
        assert!(page
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Bullet(s) if s.contains("Two"))));
    }
    #[test]
    fn accepts_only_mdn_docs() {
        assert!(normalize_url("/en-US/docs/Web/CSS").is_ok());
        assert!(normalize_url("https://example.com/en-US/docs/Web/CSS").is_err());
        assert!(normalize_url("http://developer.mozilla.org/en-US/docs/Web/CSS").is_err());
    }
    #[test]
    fn source_fallback_keeps_examples_and_placeholders() {
        let source = "---\ntitle: Media\n---\n# Media\n\nUse this rule:\n\n```css\n@media (color) {}\n```\n\n![Diagram](diagram.svg)\n\n{{EmbedLiveSample(\"media\")}}\n";
        let page = parse_markdown(
            source,
            "https://developer.mozilla.org/en-US/docs/Web/CSS",
            "Fallback",
        );
        assert_eq!(page.title, "Media");
        assert_eq!(page.examples[0].caption, "Use this rule:");
        assert_eq!(page.examples[0].language, "css");
        assert!(page
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Image(s) if s.contains("Diagram"))));
        assert!(page
            .blocks
            .iter()
            .any(|b| matches!(b, Block::Image(s) if s.contains("Interactive example"))));
    }
}
