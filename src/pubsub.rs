use chrono::DateTime;
use quick_xml::de::from_str;
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct YoutubeLink {
    #[serde(rename = "@rel")]
    pub rel: String,
    #[serde(rename = "@href")]
    pub href: String,
}

#[derive(Debug, Deserialize)]
pub struct YoutubeAuthor {
    pub name: String,
    pub uri: String,
}

#[derive(Debug, Deserialize)]
pub struct YoutubeEntry {
    pub id: String,
    #[serde(rename = "videoId")]
    pub video_id: String,
    #[serde(rename = "channelId")]
    pub channel_id: String,
    pub title: String,
    pub link: YoutubeLink,
    pub author: YoutubeAuthor,
    #[serde(deserialize_with = "de_time", default)]
    pub published: Option<DateTime<chrono::FixedOffset>>,
    #[serde(deserialize_with = "de_time", default)]
    pub updated: Option<DateTime<chrono::FixedOffset>>,
}

#[derive(Debug, Deserialize)]
pub struct YoutubeFeed {
    pub title: String,
    #[serde(default)]
    pub link: Vec<YoutubeLink>,
    #[serde(deserialize_with = "de_time", default)]
    pub updated: Option<DateTime<chrono::FixedOffset>>,
    #[serde(default)]
    pub entry: Vec<YoutubeEntry>,
}

fn de_time<'de, D>(deserializer: D) -> Result<Option<DateTime<chrono::FixedOffset>>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;

    let dt: Result<DateTime<chrono::FixedOffset>, chrono::ParseError> =
        chrono::DateTime::parse_from_rfc3339(&s);

    match dt {
        Ok(ts) => Ok(Some(ts)),
        Err(e) => {
            println!("Error parsing time ({}):  {}", s, e);
            Ok(None)
        }
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
