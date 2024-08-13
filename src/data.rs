use chrono::{DateTime, NaiveDateTime, Utc};
use futures::StreamExt;
use mongodb::bson;
use mongodb::bson::doc;
use mongodb::options::FindOptions;
use mongodb::{options::ClientOptions, Client};
use regex::Regex;
pub mod models;

pub struct Mongo {
    client: Client,
}

impl Mongo {
    pub async fn new() -> Self {
        let mut client_options = ClientOptions::parse(
            std::env::var("MONGO_CONNECTION_URL")
                .unwrap_or_else(|_| "mongodb://localhost:27017".to_string()),
        )
        .await
        .unwrap();

        client_options.app_name = Some("Having fun with MongoDB and Rust".to_string());

        let client = Client::with_options(client_options).unwrap();

        Self { client }
    }

    pub async fn get_livestream(
        &self,
        url: &str,
    ) -> mongodb::error::Result<Option<models::Livestream>> {
        let typed_collection = self
            .client
            .database("hololive-en")
            .collection::<models::Livestream>("scheduledLivestreams");

        typed_collection.find_one(doc! { "url": url }, None).await
    }

    pub async fn get_livestreams(&self) -> mongodb::error::Result<Vec<models::Livestream>> {
        let typed_collection = self
            .client
            .database("hololive-en")
            .collection::<models::Livestream>("scheduledLivestreams");
        let filter = doc! {};
        let find_options = FindOptions::builder().sort(doc! { "date": 1 }).build();
        let cursor = typed_collection.find(filter, find_options);

        let livestreams: Vec<models::Livestream> = cursor
            .await?
            .filter_map(|doc| async move {
                match doc {
                    Ok(doc) => Some(doc),
                    Err(e) => {
                        println!("Error parsing livestream: {}", e);
                        None
                    }
                }
            })
            .collect()
            .await;
        Ok(livestreams)
    }

    pub async fn insert_livestream(
        &self,
        livestream: &models::Livestream,
    ) -> mongodb::error::Result<()> {
        let typed_collection = self
            .client
            .database("hololive-en")
            .collection::<models::Livestream>("scheduledLivestreams");
        let insert_result = typed_collection.insert_one(livestream, None).await?;
        println!("Inserted livestream with id {}", insert_result.inserted_id);
        Ok(())
    }

    pub async fn upsert_livestream(
        &self,
        livestream: &models::Livestream,
    ) -> mongodb::error::Result<()> {
        let typed_collection = self
            .client
            .database("hololive-en")
            .collection::<models::Livestream>("scheduledLivestreams");
        let filter = doc! { "url": &livestream.url };
        let update = doc! { "$set": bson::to_bson(&livestream).unwrap() };
        let options = Some(
            mongodb::options::UpdateOptions::builder()
                .upsert(Some(true))
                .build(),
        );
        let update_result = typed_collection.update_one(filter, update, options).await?;

        println!("Upsert result {:?}", update_result);
        Ok(())
    }

    pub async fn get_feeds(&self) -> mongodb::error::Result<Vec<models::Feed>> {
        let typed_collection = self
            .client
            .database("hololive-en")
            .collection::<models::Feed>("feeds");
        let filter = doc! {};
        let find_options = FindOptions::builder().sort(doc! { "date": 1 }).build();
        let cursor = typed_collection.find(filter, find_options);

        let feeds: Vec<models::Feed> = cursor
            .await?
            .filter_map(|doc| async move {
                match doc {
                    Ok(doc) => Some(doc),
                    Err(e) => {
                        println!("Error parsing feed: {}", e);
                        None
                    }
                }
            })
            .collect()
            .await;
        Ok(feeds)
    }
}

pub struct Scraper {
    client: reqwest::Client,
}

impl Scraper {
    pub fn new() -> Self {
        let client = reqwest::Client::new();
        Self { client }
    }

    async fn get(&self, url: &str) -> Result<String, Box<dyn std::error::Error>> {
        let res = self.client.get(url).send().await?;

        Ok(res.text().await?)
    }

    pub async fn get_stream_dt(
        &self,
        url: &str,
    ) -> Result<DateTime<Utc>, Box<dyn std::error::Error>> {
        let html = self.get(url).await?;

        let re = Regex::new(r#"(?:"scheduledStartTime":")(?P<timestamp>\d+)"#).unwrap();

        let captures = re.captures_iter(&html).collect::<Vec<_>>();
        let capture = captures.first();

        match capture {
            Some(capture) => {
                let (_, [timestamp]) = capture.extract();

                let timestamp = timestamp.parse::<i64>().unwrap();

                Ok(DateTime::<Utc>::from_utc(
                    NaiveDateTime::from_timestamp_opt(timestamp, 0).unwrap(),
                    Utc,
                ))
            }
            None => Err(format!("No timestamp found for URL: {}", url).into()),
        }
    }
}
