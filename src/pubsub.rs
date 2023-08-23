use chrono::DateTime;
use quick_xml::de::from_str;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct YoutubeLink {
    #[serde(rename = "@rel")]
    pub rel: String,
    #[serde(rename = "@href")]
    pub href: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct YoutubeAuthor {
    pub name: String,
    pub uri: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct YoutubeEntry {
    pub id: String,
    #[serde(rename = "videoId")]
    pub video_id: String,
    #[serde(rename = "channelId")]
    pub channel_id: String,
    pub title: String,
    pub link: YoutubeLink,
    pub author: YoutubeAuthor,
    #[serde(deserialize_with = "de_time", serialize_with = "se_time", default)]
    pub published: Option<DateTime<chrono::Utc>>,
    #[serde(deserialize_with = "de_time", serialize_with = "se_time", default)]
    pub updated: Option<DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct YoutubeFeed {
    pub title: String,
    #[serde(default)]
    pub link: Vec<YoutubeLink>,
    #[serde(deserialize_with = "de_time", serialize_with = "se_time", default)]
    pub updated: Option<DateTime<chrono::Utc>>,
    #[serde(default)]
    pub entry: Vec<YoutubeEntry>,
}

/// Deserialize a `DateTime` from an RFC 3339 date string.
fn de_time<'de, D>(deserializer: D) -> Result<Option<DateTime<chrono::Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;

    let dt: Result<DateTime<chrono::FixedOffset>, chrono::ParseError> =
        chrono::DateTime::parse_from_rfc3339(&s);

    match dt {
        Ok(dt) => {
            let dt = dt.with_timezone(&chrono::Utc);
            Ok(Some(dt))
        }
        Err(e) => {
            println!("Error parsing time ({}):  {}", s, e);
            Ok(None)
        }
    }
}

/// Serialize a `DateTime` into an RFC 3339 date string as bytes.
fn se_time<S>(dt: &Option<DateTime<chrono::Utc>>, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match dt {
        Some(dt) => Ok(s.serialize_str(&dt.to_rfc3339())?),
        None => Ok(s.serialize_str("")?),
    }
}

#[derive(Debug)]
pub struct Subscription {
    hub: String,
    topic: reqwest::Url,
}

#[derive(Debug)]
pub struct PubSub {
    client: reqwest::Client,
    subscriptions: HashMap<reqwest::Url, Subscription>,
    callback_url: reqwest::Url,
}

impl PubSub {
    pub fn new(callback_url: reqwest::Url) -> Self {
        Self {
            client: reqwest::Client::new(),
            subscriptions: HashMap::new(),
            callback_url,
        }
    }

    pub async fn subscribe(&mut self, url: reqwest::Url) -> Result<(), Box<dyn std::error::Error>> {
        let hub = self.discover_hub(url.as_str()).await?;

        let mut form_data = HashMap::new();
        form_data.insert("hub.callback", self.callback_url.as_str());
        form_data.insert("hub.topic", url.as_str());
        form_data.insert("hub.mode", "subscribe");

        let response = self
            .client
            .post(&hub)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&form_data)
            .send()
            .await?;

        if response.status() != 202 {
            return Err(format!("Status code not 202. {:?}", response).into());
        }

        let url_clone = url.clone();

        self.subscriptions.insert(
            url,
            Subscription {
                hub,
                topic: url_clone,
            },
        );

        println!("Subscribe {:?}", response);
        Ok(())
    }

    pub async fn unsubscribe(
        &mut self,
        url: reqwest::Url,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let hub = self.discover_hub(url.as_str()).await?;

        let mut form_data = HashMap::new();
        form_data.insert("hub.callback", self.callback_url.as_str());
        form_data.insert("hub.topic", url.as_str());
        form_data.insert("hub.mode", "unsubscribe");

        let response = self
            .client
            .post(&hub)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&form_data)
            .send()
            .await?;

        if response.status() != 202 {
            return Err(format!("Status code not 202. {:?}", response).into());
        }

        self.subscriptions.remove(&url);

        println!("Unsubscribe {:?}", response);
        Ok(())
    }

    async fn discover_hub(&self, url: &str) -> Result<String, Box<dyn std::error::Error>> {
        let url = reqwest::Url::parse(url)?;
        let data = reqwest::get(url).await?.text().await?;

        let feed: YoutubeFeed = from_str(&data)?;

        println!("Discovered Feed {:?}", feed);

        for link in feed.link {
            if link.rel == "hub" {
                return Ok(link.href);
            }
        }

        Err("No hub found".into())
    }
}
