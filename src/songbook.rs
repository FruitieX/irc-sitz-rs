use std::fmt::{Display, Formatter};

use anyhow::{anyhow, Context, Result};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Clone, Default, Debug, Deserialize, Serialize)]
pub struct SongbookSong {
    pub id: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub book: Option<String>,
}

impl PartialEq for SongbookSong {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Display for SongbookSong {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let title = self.title.as_ref().unwrap_or(&self.id);
        let book = &self.book;

        if let Some(book) = book {
            write!(f, "{} ({})", title, book)
        } else {
            write!(f, "{}", title)
        }
    }
}

pub async fn get_song_info(url: &str, config: &Config) -> Result<SongbookSong> {
    let url_matches = config.songbook.songbook_re.captures(url).with_context(|| {
        format!(
            "URL mismatch, try pasting a URL from {}",
            config.songbook.songbook_url
        )
    })?;
    let id = url_matches
        .get(2)
        .map(|id| id.as_str().to_string())
        .context("No ID found in URL")?;

    let result = reqwest::get(url)
        .await
        .with_context(|| format!("Request to {url} failed"))?
        .error_for_status();

    let response = match result {
        Ok(response) => response,
        Err(e) => return Err(anyhow!("Failed to get songbook song info: {}", e)),
    };

    let html = response.text().await?;
    let document = Html::parse_document(&html);
    let title_selector = Selector::parse("h1").unwrap();
    let book_selector = Selector::parse("[class^=SongTags__Wrapper] > *:last-child").unwrap();

    let title = document
        .select(&title_selector)
        .next()
        .and_then(|element| element.text().next())
        .map(|text| text.to_string());

    let book = document
        .select(&book_selector)
        .next()
        .and_then(|element| element.text().next())
        .map(|text| text.to_string());

    Ok(SongbookSong {
        url: Some(url.to_string()),
        id,
        title,
        book,
    })
}
