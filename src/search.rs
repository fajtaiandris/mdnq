use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct SearchItem {
    pub title: String,
    pub url: String,
}

pub fn search(index: &[SearchItem], query: &str, limit: usize) -> Vec<SearchItem> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<(i32, &SearchItem)> = index
        .iter()
        .filter_map(|item| match_score(query, &item.title, &item.url).map(|score| (score, item)))
        .collect();
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.title.len().cmp(&b.1.title.len()))
    });
    if ranked.len() < limit {
        let mut typos: Vec<_> = index
            .iter()
            .filter(|item| !ranked.iter().any(|(_, found)| found.url == item.url))
            .filter_map(|item| typo_score(query, &item.title).map(|score| (score, item)))
            .collect();
        ranked.append(&mut typos);
        ranked.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.title.len().cmp(&b.1.title.len()))
        });
    }
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, item)| item.clone())
        .collect()
}

fn match_score(query: &str, title: &str, url: &str) -> Option<i32> {
    let q = query.trim().to_lowercase();
    let title = title.to_lowercase();
    let path = url.to_lowercase().replace(['_', '/'], " ");
    if title == q {
        return Some(1000);
    }
    if title.starts_with(&q) {
        return Some(850 - (title.len() - q.len()).min(100) as i32);
    }
    if let Some(pos) = title.find(&q) {
        return Some(700 - pos as i32);
    }
    let words: Vec<_> = q.split_whitespace().collect();
    if words
        .iter()
        .all(|word| title.split_whitespace().any(|part| part.contains(word)))
    {
        return Some(600 - (title.len() / 10) as i32);
    }
    if words.iter().all(|word| path.contains(word)) {
        return Some(400 - (path.len() / 10) as i32);
    }
    let mut chars = title.chars();
    let mut gaps = 0;
    for needle in q.chars().filter(|c| !c.is_whitespace()) {
        let mut found = false;
        for candidate in chars.by_ref() {
            if candidate == needle {
                found = true;
                break;
            }
            gaps += 1;
        }
        if !found {
            return None;
        }
    }
    Some(250 - gaps * 3 - ((title.len().saturating_sub(q.len())) / 5) as i32)
}

fn typo_score(query: &str, title: &str) -> Option<i32> {
    let query_words: Vec<_> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let title_words: Vec<_> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    if query_words.is_empty() || query_words.iter().any(|w| w.chars().count() < 4) {
        return None;
    }
    let mut total = 0;
    for word in query_words {
        let distance = title_words
            .iter()
            .map(|candidate| edit_distance(word, candidate))
            .min()
            .unwrap_or(3);
        if distance > 2 {
            return None;
        }
        total += distance;
    }
    if total == 0 {
        None
    } else {
        Some(180 - total * 30 - (title.len() / 3) as i32)
    }
}

fn edit_distance(a: &str, b: &str) -> i32 {
    let a: Vec<char> = a.to_lowercase().chars().collect();
    let b: Vec<char> = b.to_lowercase().chars().collect();
    if a.len().abs_diff(b.len()) > 2 {
        return 3;
    }
    let mut previous: Vec<i32> = (0..=b.len() as i32).collect();
    let mut previous_previous = previous.clone();
    for (i, &left) in a.iter().enumerate() {
        let mut current = vec![(i + 1) as i32];
        for (j, &right) in b.iter().enumerate() {
            let mut value = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + i32::from(left != right));
            if i > 0 && j > 0 && left == b[j - 1] && a[i - 1] == right {
                value = value.min(previous_previous[j - 1] + 1);
            }
            current.push(value);
        }
        if *current.iter().min().unwrap_or(&3) > 2 {
            return 3;
        }
        previous_previous = previous;
        previous = current;
    }
    previous[b.len()].min(3)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranks_titles_and_recovers_typo() {
        let index = vec![
            SearchItem {
                title: "Using media queries".into(),
                url: "/en-US/docs/Web/CSS/Guides/Media_queries/Using".into(),
            },
            SearchItem {
                title: "@media".into(),
                url: "/en-US/docs/Web/CSS/Reference/At-rules/@media".into(),
            },
        ];
        assert_eq!(
            search(&index, "using media", 10)[0].title,
            "Using media queries"
        );
        assert_eq!(
            search(&index, "medai quries", 10)[0].title,
            "Using media queries"
        );
    }
}
