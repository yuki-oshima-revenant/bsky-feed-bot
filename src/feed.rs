use std::collections::HashMap;

use aws_sdk_dynamodb::types::AttributeValue;
use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use feed_rs::model::Feed;
use scraper::{Html, Selector};

use crate::OpaqueError;

fn get_string_from_attribute_value_map(
    map: &HashMap<String, AttributeValue>,
    key: &str,
) -> Result<String, OpaqueError> {
    let value = map
        .get(key)
        .ok_or(format!("no {}", key))?
        .as_s()
        .map_err(|v| format!("invalid {}, {:?}", key, v))?;
    Ok(value.clone())
}

pub struct RegisteredFeed {
    pub url: String,
}

pub async fn list_registered_feeds(
    dynamodb_client: &aws_sdk_dynamodb::Client,
) -> Result<Vec<RegisteredFeed>, OpaqueError> {
    let scan_output = dynamodb_client
        .scan()
        .table_name("bsky-feed-bot-registered-feeds")
        .select(aws_sdk_dynamodb::types::Select::AllAttributes)
        .send()
        .await?;
    let items: Vec<HashMap<String, AttributeValue>> = scan_output.items.ok_or("no items")?;
    let registered_feeds: Vec<RegisteredFeed> = items
        .iter()
        .map(|item| {
            let url = get_string_from_attribute_value_map(item, "url")?;
            Ok(RegisteredFeed { url })
        })
        .collect::<Result<Vec<RegisteredFeed>, OpaqueError>>()?;
    Ok(registered_feeds)
}

pub async fn get_feed(feed_url: &str) -> Result<Feed, OpaqueError> {
    let response = reqwest::get(feed_url).await?;
    let bytes = response.bytes().await?;
    let feed = feed_rs::parser::parse(bytes.to_vec().as_slice())?;
    Ok(feed)
}

pub fn is_date_in_past_range(
    event_time: DateTime<Utc>,
    target_time: Option<DateTime<Utc>>,
    duration: Duration,
) -> bool {
    let target_time = match target_time {
        Some(target_time) => target_time,
        None => return false,
    };
    let start_datetime = event_time - duration;
    target_time > start_datetime && target_time <= event_time
}

#[derive(Debug, Clone)]
pub struct FeedEntry {
    pub url: String,
    pub title: Option<String>,
    pub published: Option<DateTime<Utc>>,
}

pub fn extract_feed_entries(feed: Feed) -> Vec<FeedEntry> {
    let mut entries = Vec::new();
    for entry in feed.entries {
        if let Some(link) = entry.links.get(0) {
            let title = entry
                .title
                .and_then(|title_element| Some(title_element.content));
            entries.push(FeedEntry {
                url: link.href.clone(),
                title,
                published: entry.published,
            });
        }
    }
    entries
}

#[derive(Debug)]
pub struct OGPInfo {
    pub title: Option<String>,
    pub image_url: Option<String>,
    pub description: Option<String>,
}

pub async fn get_ogp_from_url(url: &str) -> Result<OGPInfo, OpaqueError> {
    let response = reqwest::get(url).await?;
    let text = response.text().await?;
    let html = Html::parse_document(&text);
    let title = extract_ogp_info_from_meta_tag(&html, "og:title");
    let image_url = extract_ogp_info_from_meta_tag(&html, "og:image");
    let description = extract_ogp_info_from_meta_tag(&html, "og:description");
    Ok(OGPInfo {
        title: title.map(|s| s.to_string()),
        image_url: image_url.map(|s| s.to_string()),
        description: description.map(|s| s.to_string()),
    })
}

fn extract_ogp_info_from_meta_tag<'a>(html: &'a Html, property: &str) -> Option<&'a str> {
    if let Ok(selector) = Selector::parse(&format!(r#"meta[property="{property}"]"#)) {
        let mut meta_tag = html.select(&selector);
        if let Some(tag) = meta_tag.next() {
            return tag.value().attr("content");
        };
    };
    None
}

#[derive(Debug)]
pub struct OGImage {
    pub image: Bytes,
    pub content_type: String,
}

pub async fn get_og_image(image_url: &str) -> Result<OGImage, OpaqueError> {
    let response = reqwest::get(image_url).await?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .unwrap()
        .to_str()?
        .to_string();
    let bytes = response.bytes().await?;
    Ok(OGImage {
        image: bytes,
        content_type,
    })
}

pub async fn extract_feed_entry_info(
    feed_entry: &FeedEntry,
) -> Result<(Option<OGPInfo>, Option<OGImage>), OpaqueError> {
    let ogp_info = get_ogp_from_url(&feed_entry.url).await.ok();
    let og_image = match &ogp_info {
        Some(OGPInfo {
            image_url: Some(image_url),
            ..
        }) => get_og_image(image_url).await.ok(),
        _ => None,
    };
    Ok((ogp_info, og_image))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_rss_feed() {
        let feed = get_feed("https://zed.dev/blog.rss").await.unwrap();
        let entries = extract_feed_entries(feed);
        let entry = entries.get(0).unwrap();
        println!("{:?}", entry);
        let ogp_info = get_ogp_from_url(&entry.url).await.unwrap();
        println!("{:?}", ogp_info);
        let og_image = get_og_image(&ogp_info.image_url.unwrap()).await.unwrap();
        println!("{:?}", og_image);
    }

    #[tokio::test]
    async fn test_get_atom_feed() {
        let feed = get_feed("https://blog.rust-lang.org/feed.xml")
            .await
            .unwrap();
        let entries = extract_feed_entries(feed);
        let entry = entries.get(0).unwrap();
        println!("{:?}", entry);
        let ogp_info = get_ogp_from_url(&entry.url).await.unwrap();
        println!("{:?}", ogp_info);
        let og_image = get_og_image(&ogp_info.image_url.unwrap()).await.unwrap();
        println!("{:?}", og_image);
    }
}
