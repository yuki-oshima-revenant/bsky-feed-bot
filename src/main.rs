use aws_config::BehaviorVersion;
use aws_lambda_events::eventbridge::EventBridgeEvent;
use bsky::BskyClient;
use dynamodb::{list_registered_feeds, FeedRecord};
use feed::{extract_feed_entries, extract_feed_entry_info, get_feed};
use lambda_runtime::{service_fn, LambdaEvent};

use crate::dynamodb::update_application_info_in_dynamodb;

mod bsky;
mod dynamodb;
mod feed;

pub type OpaqueError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    lambda_runtime::run(service_fn(lambda_handler)).await?;
    Ok(())
}

async fn lambda_handler(
    _: LambdaEvent<EventBridgeEvent<serde_json::Value>>,
) -> Result<(), lambda_runtime::Error> {
    match execute().await {
        Ok(_) => Ok(()),
        Err(err) => {
            println!("Error: {:?}", err);
            Err(err.into())
        }
    }
}

async fn execute() -> Result<Vec<()>, OpaqueError> {
    let aws_config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let dynamodb_client = aws_sdk_dynamodb::Client::new(&aws_config);
    let mut bsky_client = bsky::BskyClient::new().await?;
    let feed_records = list_registered_feeds(&dynamodb_client).await?;
    let mut feed_process_results = Vec::new();
    // todo: process feeds concurrently
    for feed_record in feed_records {
        let feed_process_result =
            process_feed(&feed_record, &mut bsky_client, &dynamodb_client).await;
        feed_process_results.push(feed_process_result);
    }
    let result = feed_process_results
        .into_iter()
        .collect::<Result<Vec<()>, OpaqueError>>()?;
    Ok(result)
}

async fn process_feed(
    feed_record: &FeedRecord,
    bsky_client: &mut BskyClient,
    dynamodb_client: &aws_sdk_dynamodb::Client,
) -> Result<(), OpaqueError> {
    println!("Processing feed: {}", feed_record.url);
    let feed = get_feed(&feed_record.url).await?;
    let entries = extract_feed_entries(&feed);
    let mut target_entries = Vec::new();
    for (index, feed_entry) in entries.iter().enumerate() {
        if let Some(last_posted_entry_id) = &feed_record.last_posted_entry_id {
            if feed_entry.id == *last_posted_entry_id {
                break;
            }
        }
        target_entries.push(feed_entry.clone());
        // last_posted_entry_idが登録されていない場合は最新の1件を投稿する
        if index == 0 && feed_record.last_posted_entry_id.is_none() {
            break;
        }
    }
    target_entries.reverse();
    let mut last_posted_entry_id: Option<String> = feed_record.last_posted_entry_id.clone();
    for feed_entry in target_entries {
        println!("Processing entry: {}", feed_entry.id);
        let (ogp_info, og_image) = extract_feed_entry_info(&feed_entry).await?;
        let upload_blog_response = match og_image {
            Some(og_image) => Some(
                bsky_client
                    .upload_thumbnail_with_resizing(og_image.image)
                    .await?,
            ),
            None => None,
        };
        let create_record_request = bsky_client
            .format_create_record_request_from_feed_entry(
                &feed,
                feed_entry.clone(),
                ogp_info,
                upload_blog_response,
            )
            .await;
        bsky_client.create_record(create_record_request).await?;
        last_posted_entry_id = Some(feed_entry.id.clone());
    }
    if let Some(last_posted_entry_id) = last_posted_entry_id {
        update_application_info_in_dynamodb(
            dynamodb_client,
            &feed_record.url,
            &last_posted_entry_id,
        )
        .await?;
    }
    println!("Finished processing feed: {}", feed_record.url);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotenvy::dotenv;

    #[tokio::test]
    async fn test_execute() {
        dotenv().ok();
        execute().await.unwrap();
    }

    #[tokio::test]
    async fn test_process_feed() {
        dotenv().ok();
        let aws_config = aws_config::load_defaults(BehaviorVersion::latest()).await;
        let dynamodb_client = aws_sdk_dynamodb::Client::new(&aws_config);
        let mut bsky_client = bsky::BskyClient::new().await.unwrap();
        let feed_record = FeedRecord {
            url: "https://blog.rust-lang.org/feed.xml".to_string(),
            last_posted_entry_id: Some(
                "https://blog.rust-lang.org/2023/12/28/Rust-1.75.0.html".to_string(),
            ),
        };
        process_feed(&feed_record, &mut bsky_client, &dynamodb_client)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_process_feed_no_last_posted_entry_id() {
        dotenv().ok();
        let aws_config = aws_config::load_defaults(BehaviorVersion::latest()).await;
        let dynamodb_client = aws_sdk_dynamodb::Client::new(&aws_config);
        let mut bsky_client = bsky::BskyClient::new().await.unwrap();
        let feed_record = FeedRecord {
            url: "https://blog.rust-lang.org/feed.xml".to_string(),
            last_posted_entry_id: None,
        };
        process_feed(&feed_record, &mut bsky_client, &dynamodb_client)
            .await
            .unwrap();
    }
}
