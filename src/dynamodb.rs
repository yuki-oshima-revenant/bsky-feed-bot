use std::collections::HashMap;

use aws_sdk_dynamodb::{operation::update_item::UpdateItemOutput, types::AttributeValue};

use crate::OpaqueError;

static TABLE_NAME: &str = "bsky-feed-bot-registered-feeds";

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

fn get_optional_string_from_attribute_value_map(
    map: &HashMap<String, AttributeValue>,
    key: &str,
) -> Result<Option<String>, OpaqueError> {
    let value = map.get(key).and_then(|v| v.as_s().ok()).map(|s| s.clone());
    Ok(value)
}

pub struct FeedRecord {
    pub url: String,
    pub last_posted_entry_id: Option<String>,
}

pub async fn list_registered_feeds(
    dynamodb_client: &aws_sdk_dynamodb::Client,
) -> Result<Vec<FeedRecord>, OpaqueError> {
    let scan_output = dynamodb_client
        .scan()
        .table_name(TABLE_NAME)
        .select(aws_sdk_dynamodb::types::Select::AllAttributes)
        .send()
        .await?;
    let items: Vec<HashMap<String, AttributeValue>> = scan_output.items.ok_or("no items")?;
    let registered_feeds: Vec<FeedRecord> = items
        .iter()
        .map(|item| {
            let url = get_string_from_attribute_value_map(item, "url")?;
            let last_posted_entry_id =
                get_optional_string_from_attribute_value_map(item, "last_posted_entry_id")?;
            Ok(FeedRecord {
                url,
                last_posted_entry_id,
            })
        })
        .collect::<Result<Vec<FeedRecord>, OpaqueError>>()?;
    Ok(registered_feeds)
}

pub async fn update_application_info_in_dynamodb(
    dynamodb_client: &aws_sdk_dynamodb::Client,
    feed_url: &str,
    last_posted_entry_id: &str,
) -> Result<UpdateItemOutput, OpaqueError> {
    let update_output = dynamodb_client
        .update_item()
        .table_name(TABLE_NAME)
        .key("url", AttributeValue::S(feed_url.to_string()))
        .update_expression("SET last_posted_entry_id = :last_posted_entry_id")
        .expression_attribute_values(
            ":last_posted_entry_id",
            AttributeValue::S(last_posted_entry_id.to_string()),
        )
        .send()
        .await?;
    Ok(update_output)
}
