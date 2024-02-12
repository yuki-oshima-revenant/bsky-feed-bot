use feed::{get_og_image, get_ogp_from_url, FeedEntry, OGImage, OGPInfo};

mod bsky;
mod feed;

pub type OpaqueError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[tokio::main]
async fn main() {
    println!("Hello, world!");
}

async fn extract_feed_entry_info(
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
    use dotenvy::dotenv;
    use feed::{extract_entries_info, get_feed};

    #[tokio::test]
    async fn test_post_feed_entry() {
        dotenv().ok();
        let feed = get_feed("https://blog.rust-lang.org/feed.xml")
            .await
            .unwrap();
        let entries = extract_entries_info(feed);
        let feed_entry = entries.get(0).unwrap();
        let (ogp_info, og_image) = extract_feed_entry_info(&feed_entry).await.unwrap();
        let bsky_client = bsky::BskyClient::new().await.unwrap();
        let upload_blog_response = match og_image {
            Some(og_image) => Some(bsky_client.upload_blob(og_image.image).await.unwrap()),
            None => None,
        };
        let create_record_request = bsky_client
            .format_create_record_request_from_feed_entry(
                feed_entry.clone(),
                ogp_info,
                upload_blog_response,
            )
            .await;
        println!("{:?}", create_record_request);
        let response = bsky_client
            .create_record(create_record_request)
            .await
            .unwrap();
        println!("{:?}", response);
    }
}
