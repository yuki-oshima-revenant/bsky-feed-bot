use aws_config::BehaviorVersion;
use aws_lambda_events::eventbridge::EventBridgeEvent;
use bsky::BskyClient;
use chrono::{DateTime, Duration, Utc};
use feed::{extract_feed_entries, extract_feed_entry_info, get_feed, is_date_in_past_range};
use lambda_runtime::{service_fn, LambdaEvent};

use crate::feed::list_registered_feeds;

mod bsky;
mod feed;

pub type OpaqueError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    lambda_runtime::run(service_fn(lambda_handler)).await?;
    Ok(())
}

async fn lambda_handler(
    event: LambdaEvent<EventBridgeEvent<serde_json::Value>>,
) -> Result<(), lambda_runtime::Error> {
    let event_time = match event.payload.time {
        Some(time) => time,
        None => {
            return Err("Event time not found".into());
        }
    };
    match execute(&event_time).await {
        Ok(_) => Ok(()),
        Err(err) => {
            println!("Error: {:?}", err);
            Err(err.into())
        }
    }
}

async fn execute(event_time: &DateTime<Utc>) -> Result<Vec<()>, OpaqueError> {
    let aws_config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let dynamodb_client = aws_sdk_dynamodb::Client::new(&aws_config);
    let mut bsky_client = bsky::BskyClient::new().await?;
    let registered_feeds = list_registered_feeds(&dynamodb_client).await?;
    let mut feed_process_results = Vec::new();
    // todo: process feeds concurrently
    for feed in registered_feeds {
        let feed_process_result = process_feed(&event_time, &feed.url, &mut bsky_client).await;
        feed_process_results.push(feed_process_result);
    }
    let result = feed_process_results
        .into_iter()
        .collect::<Result<Vec<()>, OpaqueError>>()?;
    Ok(result)
}

async fn process_feed(
    event_time: &DateTime<Utc>,
    feed_url: &str,
    bsky_client: &mut BskyClient,
) -> Result<(), OpaqueError> {
    println!("Processing feed: {}", feed_url);
    let feed: feed_rs::model::Feed = get_feed(feed_url).await?;
    let entries = extract_feed_entries(feed);
    for feed_entry in entries {
        if !is_date_in_past_range(
            event_time.clone(),
            feed_entry.published.clone(),
            Duration::days(1),
        ) {
            continue;
        }
        let (ogp_info, og_image) = extract_feed_entry_info(&feed_entry).await?;
        let upload_blog_response = match og_image {
            Some(og_image) => Some(bsky_client.upload_blob(og_image.image).await?),
            None => None,
        };
        let create_record_request = bsky_client
            .format_create_record_request_from_feed_entry(
                feed_entry.clone(),
                ogp_info,
                upload_blog_response,
            )
            .await;
        bsky_client.create_record(create_record_request).await?;
    }
    println!("Finished processing feed: {}", feed_url);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dotenvy::dotenv;

    #[tokio::test]
    async fn test_execute() {
        dotenv().ok();
        let event_time = DateTime::parse_from_rfc3339("2024-02-08T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        execute(&event_time).await.unwrap();
    }
}
