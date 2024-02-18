use std::{env, io::Cursor};

use bytes::Bytes;
use chrono::{SecondsFormat, Utc};
use feed_rs::model::Feed;
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
    #[serde(skip_serializing_if = "Option::is_none")]
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
    #[serde(skip_serializing_if = "Option::is_none")]
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

    pub async fn refresh_session(&mut self) -> Result<(), OpaqueError> {
        let mut headers = HeaderMap::new();
        headers.append(header::ACCEPT, HeaderValue::from_static("application/json"));
        let response = self
            .reqwest_client
            .post("https://bsky.social/xrpc/com.atproto.server.refreshSession")
            .bearer_auth(&self.session.refresh_jwt)
            .headers(headers)
            .send()
            .await?
            .error_for_status()?;
        let session: Session = response.json().await?;
        self.session = session;
        Ok(())
    }

    async fn execute_request_with_refresh_session(
        &mut self,
        request: reqwest::Request,
    ) -> Result<reqwest::Response, OpaqueError> {
        let mut response = self
            .reqwest_client
            .execute(request.try_clone().ok_or("Failed to clone request")?)
            .await?;
        if response.status() == 401 {
            self.refresh_session().await?;
            response = self
                .reqwest_client
                .execute(request)
                .await?
                .error_for_status()?;
        } else {
            response = response.error_for_status()?;
        }
        Ok(response)
    }

    async fn upload_blob(&mut self, body: Bytes) -> Result<UploadBlobResponse, OpaqueError> {
        let mut headers = HeaderMap::new();
        headers.append(header::CONTENT_TYPE, HeaderValue::from_static("*/*"));
        headers.append(header::ACCEPT, HeaderValue::from_static("application/json"));
        let request = self
            .reqwest_client
            .post("https://bsky.social/xrpc/com.atproto.repo.uploadBlob")
            .bearer_auth(&self.session.access_jwt)
            .headers(headers)
            .body(body)
            .build()?;
        let response = self.execute_request_with_refresh_session(request).await?;
        let response_body: UploadBlobResponse = response.json().await?;
        Ok(response_body)
    }

    pub async fn upload_thumbnail_with_resizing(
        &mut self,
        image_bytes: Bytes,
    ) -> Result<UploadBlobResponse, OpaqueError> {
        let resized_image_bytes = match resize_thumbnail(&image_bytes) {
            Ok(resized_image_bytes) => resized_image_bytes,
            Err(_) => image_bytes,
        };
        self.upload_blob(resized_image_bytes).await
    }

    pub async fn create_record(
        &mut self,
        request: CreateRecordRequest,
    ) -> Result<CreateRecordResponse, OpaqueError> {
        let mut headers = HeaderMap::new();
        headers.append(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers.append(header::ACCEPT, HeaderValue::from_static("application/json"));
        let request = self
            .reqwest_client
            .post("https://bsky.social/xrpc/com.atproto.repo.createRecord")
            .bearer_auth(&self.session.access_jwt)
            .headers(headers)
            .body(serde_json::to_string(&request)?)
            .build()?;
        let response = self.execute_request_with_refresh_session(request).await?;
        let response_body: CreateRecordResponse = response.json().await?;
        Ok(response_body)
    }

    pub async fn format_create_record_request_from_feed_entry(
        &self,
        feed: &Feed,
        feed_entry: FeedEntry,
        ogp_info: Option<OGPInfo>,
        upload_blob_response: Option<UploadBlobResponse>,
    ) -> CreateRecordRequest {
        let mut title = match &feed_entry.title {
            Some(entry_title) => match &feed.title {
                Some(feed_title) => format!("{} | {}", entry_title, feed_title.content),
                None => entry_title.clone(),
            },
            None => "".to_string(),
        };
        if cfg!(debug_assertions) {
            title = format!("[test]\n{}", title);
        }
        let thumb = match upload_blob_response {
            Some(upload_blob_response) => {
                if upload_blob_response.blob.size > 1000000 {
                    None
                } else {
                    Some(upload_blob_response.blob)
                }
            }
            None => None,
        };

        let embed = match ogp_info {
            Some(ogp_info) => {
                let embed_title = if let Some(ogp_title) = ogp_info.title {
                    ogp_title
                } else if let Some(feed_entry_title) = feed_entry.title {
                    feed_entry_title
                } else {
                    "".to_string()
                };
                Some(Embed {
                    r#type: "app.bsky.embed.external".to_string(),
                    external: EmbedExternal {
                        uri: feed_entry.url,
                        title: embed_title,
                        description: ogp_info.description.unwrap_or("".to_string()),
                        thumb,
                    },
                })
            }
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

fn resize_thumbnail(image_bytes: &Bytes) -> Result<Bytes, OpaqueError> {
    let image = image::io::Reader::new(Cursor::new(image_bytes))
        .with_guessed_format()?
        .decode()?;
    let resized_image = image.resize(1000, 1000, image::imageops::FilterType::Lanczos3);
    let mut resized_image_bytes = Vec::new();
    resized_image.write_to(
        &mut Cursor::new(&mut resized_image_bytes),
        image::ImageOutputFormat::Jpeg(100),
    )?;
    let resized_image_bytes = Bytes::from(resized_image_bytes);
    Ok(resized_image_bytes)
}

#[cfg(test)]
mod tests {
    use crate::feed::{extract_feed_entries, extract_feed_entry_info, get_feed, get_og_image};

    use super::*;
    use dotenvy::dotenv;

    #[tokio::test]
    async fn test_create_session() {
        dotenv().ok();
        let client = BskyClient::new().await.unwrap();
        println!("{:?}", client.session);
    }

    #[tokio::test]
    async fn test_refresh_session() {
        dotenv().ok();
        let mut client = BskyClient::new().await.unwrap();
        client.refresh_session().await.unwrap();
        println!("{:?}", client.session);
    }

    #[tokio::test]
    async fn test_upload_thumbnail() {
        dotenv().ok();
        let og_image = get_og_image("https://www.rust-lang.org/static/images/rust-social-wide.jpg")
            .await
            .unwrap();
        let mut client = BskyClient::new().await.unwrap();
        let response = client
            .upload_thumbnail_with_resizing(og_image.image)
            .await
            .unwrap();
        println!("{:?}", response);
    }

    #[tokio::test]
    async fn test_create_record_request() {
        dotenv().ok();
        let feed = get_feed("https://this-week-in-rust.org/atom.xml")
            .await
            .unwrap();
        let entries = extract_feed_entries(&feed);
        let feed_entry = entries.get(0).unwrap();
        let (ogp_info, og_image) = extract_feed_entry_info(&feed_entry).await.unwrap();
        let mut bsky_client = BskyClient::new().await.unwrap();
        let upload_blog_response = match og_image {
            Some(og_image) => Some(
                bsky_client
                    .upload_thumbnail_with_resizing(og_image.image)
                    .await
                    .unwrap(),
            ),
            None => None,
        };
        println!("{:?}", upload_blog_response);
        let create_record_request = bsky_client
            .format_create_record_request_from_feed_entry(
                &feed,
                feed_entry.clone(),
                ogp_info,
                upload_blog_response,
            )
            .await;
        println!("{:?}", create_record_request);
    }

    #[tokio::test]
    async fn test_post_feed_entry() {
        dotenv().ok();
        let feed = get_feed("https://github.blog/feed/").await.unwrap();
        let entries = extract_feed_entries(&feed);
        let feed_entry = entries.get(0).unwrap();
        let (ogp_info, og_image) = extract_feed_entry_info(&feed_entry).await.unwrap();
        let mut bsky_client = BskyClient::new().await.unwrap();
        let upload_blog_response = match og_image {
            Some(og_image) => Some(
                bsky_client
                    .upload_thumbnail_with_resizing(og_image.image)
                    .await
                    .unwrap(),
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
        println!("{:?}", create_record_request);
        let response = bsky_client
            .create_record(create_record_request)
            .await
            .unwrap();
        println!("{:?}", response);
    }
}
