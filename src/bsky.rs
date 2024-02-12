use std::env;

use bytes::Bytes;
use chrono::{SecondsFormat, Utc};
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

use crate::{
    feed::{FeedEntry, OGPInfo},
    OpaqueError,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateSessionRequest {
    identifier: String,
    password: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Session {
    access_jwt: String,
    refresh_jwt: String,
    handle: String,
    did: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CreateRecordRequest {
    repo: String,
    collection: String,
    record: Record,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    #[serde(rename = "$type")]
    r#type: String,
    text: String,
    embed: Option<Embed>,
    created_at: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Embed {
    #[serde(rename = "$type")]
    r#type: String,
    external: EmbedExternal,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct EmbedExternal {
    uri: String,
    title: String,
    description: String,
    thumb: Option<Blob>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Blob {
    #[serde(rename = "$type")]
    r#type: String,
    r#ref: Ref,
    mime_type: String,
    size: u64,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Ref {
    #[serde(rename = "$link")]
    link: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CreateRecordResponse {
    pub uri: String,
    pub cid: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UploadBlobResponse {
    blob: Blob,
}

pub struct BskyClient {
    reqwest_client: reqwest::Client,
    session: Session,
}

impl BskyClient {
    pub async fn new() -> Result<Self, OpaqueError> {
        let reqwest_client = reqwest::Client::new();
        let request = CreateSessionRequest {
            identifier: env::var("BSKY_IDENTIFIER")?,
            password: env::var("BSKY_PASSWORD")?,
        };
        let mut headers = HeaderMap::new();
        headers.append(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers.append(header::ACCEPT, HeaderValue::from_static("application/json"));
        let response = reqwest_client
            .post("https://bsky.social/xrpc/com.atproto.server.createSession")
            .headers(headers)
            .body(serde_json::to_string(&request)?)
            .send()
            .await?
            .error_for_status()?;
        let session: Session = response.json().await?;
        Ok(Self {
            reqwest_client,
            session,
        })
    }

    pub async fn upload_blob(&self, body: Bytes) -> Result<UploadBlobResponse, OpaqueError> {
        let mut headers = HeaderMap::new();
        headers.append(header::CONTENT_TYPE, HeaderValue::from_static("*/*"));
        headers.append(header::ACCEPT, HeaderValue::from_static("application/json"));
        let response = self
            .reqwest_client
            .post("https://bsky.social/xrpc/com.atproto.repo.uploadBlob")
            .bearer_auth(&self.session.access_jwt)
            .headers(headers)
            .body(body)
            .send()
            .await?
            .error_for_status()?;
        let response_body: UploadBlobResponse = response.json().await?;
        Ok(response_body)
    }

    pub async fn create_record(
        &self,
        request: CreateRecordRequest,
    ) -> Result<CreateRecordResponse, OpaqueError> {
        let mut headers = HeaderMap::new();
        headers.append(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers.append(header::ACCEPT, HeaderValue::from_static("application/json"));
        let response = self
            .reqwest_client
            .post("https://bsky.social/xrpc/com.atproto.repo.createRecord")
            .bearer_auth(&self.session.access_jwt)
            .headers(headers)
            .body(serde_json::to_string(&request)?)
            .send()
            .await?
            .error_for_status()?;
        let response_body: CreateRecordResponse = response.json().await?;
        Ok(response_body)
    }

    pub async fn format_create_record_request_from_feed_entry(
        &self,
        feed_entry: FeedEntry,
        ogp_info: Option<OGPInfo>,
        upload_blob_response: Option<UploadBlobResponse>,
    ) -> CreateRecordRequest {
        let mut title = match feed_entry.title {
            Some(entry_title) => entry_title,
            None => "".to_string(),
        };
        if cfg!(debug_assertions) {
            title = format!("[test]\n{}", title);
        }
        let thumb = match upload_blob_response {
            Some(upload_blob_response) => Some(upload_blob_response.blob),
            None => None,
        };
        let embed = match ogp_info {
            Some(ogp_info) => Some(Embed {
                r#type: "app.bsky.embed.external".to_string(),
                external: EmbedExternal {
                    uri: feed_entry.url,
                    title: ogp_info.title.unwrap_or("".to_string()),
                    description: ogp_info.description.unwrap_or("".to_string()),
                    thumb,
                },
            }),
            None => None,
        };
        let created_at = Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true);
        CreateRecordRequest {
            repo: self.session.did.clone(),
            collection: "app.bsky.feed.post".to_string(),
            record: Record {
                r#type: "app.bsky.feed.post".to_string(),
                text: title,
                created_at,
                embed,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::feed::get_og_image;

    use super::*;
    use dotenvy::dotenv;

    #[tokio::test]
    async fn test_create_session() {
        dotenv().ok();
        let client = BskyClient::new().await.unwrap();
        println!("{:?}", client.session);
    }

    #[tokio::test]
    async fn test_upload_thumbnail() {
        dotenv().ok();
        let og_image = get_og_image("https://www.rust-lang.org/static/images/rust-social-wide.jpg")
            .await
            .unwrap();
        let client = BskyClient::new().await.unwrap();
        let response = client.upload_blob(og_image.image).await.unwrap();
        println!("{:?}", response);
    }
}
