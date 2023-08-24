use axum::{
    body::Body,
    extract::{Path, Query},
    http::Request,
    http::StatusCode,
    routing::{get, post},
    Extension, Router,
};
use chrono::DateTime;
use futures::{lock::Mutex, Future};
use quick_xml::de::from_str;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::HashMap, pin::Pin, sync::Arc};

#[derive(Debug, Serialize, Deserialize)]
pub struct Link {
    #[serde(rename = "@rel")]
    pub rel: String,
    #[serde(rename = "@href")]
    pub href: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    pub uri: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    #[serde(rename = "videoId")]
    pub video_id: String,
    #[serde(rename = "channelId")]
    pub channel_id: String,
    pub title: String,
    pub link: Link,
    pub author: Author,
    #[serde(deserialize_with = "de_time", serialize_with = "se_time", default)]
    pub published: Option<DateTime<chrono::Utc>>,
    #[serde(deserialize_with = "de_time", serialize_with = "se_time", default)]
    pub updated: Option<DateTime<chrono::Utc>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Feed {
    pub title: String,
    #[serde(default)]
    pub link: Vec<Link>,
    #[serde(deserialize_with = "de_time", serialize_with = "se_time", default)]
    pub updated: Option<DateTime<chrono::Utc>>,
    #[serde(default)]
    pub entry: Vec<Entry>,
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

pub type CallbackHandler =
    Box<dyn FnOnce(String) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send>;

// #[derive(Debug)]
pub struct Subscription {
    topic: reqwest::Url,
    handler: CallbackHandler,
}

type SubscriptionID = u64;

// #[derive(Debug)]
pub struct PubSub {
    client: reqwest::Client,
    subscriptions: Arc<Mutex<HashMap<SubscriptionID, Subscription>>>,
    callback_path: String,
    subscription_id: SubscriptionID,
}

impl PubSub {
    pub fn new(callback_path: &str) -> Self {
        let subscriptions = Arc::new(Mutex::new(HashMap::new()));
        // tracing::debug!("listening on {}", addr);
        Self {
            client: reqwest::Client::new(),
            subscriptions,
            callback_path: callback_path.to_string(),
            subscription_id: 0,
        }
    }

    pub async fn router(&self) -> Router {
        let router = Router::new()
            .route("/", get(default_handler))
            .route(
                format!("{}/:subscription_id", self.callback_path).as_str(),
                get(pubsub_challenge_handler),
            )
            .route(
                format!("{}/:subscription_id", self.callback_path).as_str(),
                post(pubsub_callback_handler),
            )
            .layer(Extension(Arc::clone(&self.subscriptions)));

        router
    }

    pub async fn subscribe(
        &mut self,
        url: reqwest::Url,
        handler: CallbackHandler,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let subscription_id = self.get_next_subscription_id();
        let hub = self.discover_hub(url.as_str()).await?;

        let mut form_data = HashMap::new();
        let pubsub_callback_url = self.get_pubsub_callback_url(subscription_id);
        form_data.insert("hub.callback", pubsub_callback_url.as_str());
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

        self.subscriptions.lock().await.insert(
            subscription_id,
            Subscription {
                topic: url,
                handler,
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
        let subscription_id = self.get_subscription_id(&url).await;

        if subscription_id.is_none() {
            return Err("Subscription not found".into());
        }

        let subscription_id = subscription_id.unwrap();

        let mut form_data = HashMap::new();
        let pubsub_callback_url = self.get_pubsub_callback_url(subscription_id);
        form_data.insert("hub.callback", pubsub_callback_url.as_str());
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

        if let Some(subscription_id) = self.get_subscription_id(&url).await {
            self.subscriptions.lock().await.remove(&subscription_id);
        }

        println!("Unsubscribe {:?}", response);
        Ok(())
    }

    async fn discover_hub(&self, url: &str) -> Result<String, Box<dyn std::error::Error>> {
        let url = reqwest::Url::parse(url)?;
        let data = reqwest::get(url).await?.text().await?;

        let feed: Feed = from_str(&data)?;

        println!("Discovered Feed {:?}", feed);

        for link in feed.link {
            if link.rel == "hub" {
                return Ok(link.href);
            }
        }

        Err("No hub found".into())
    }

    fn get_next_subscription_id(&mut self) -> SubscriptionID {
        self.subscription_id += 1;
        self.subscription_id
    }

    async fn get_subscription_id(&self, url: &reqwest::Url) -> Option<SubscriptionID> {
        for (id, subscription) in self.subscriptions.lock().await.iter() {
            if subscription.topic == *url {
                return Some(*id);
            }
        }

        None
    }

    fn get_pubsub_callback_url(&self, subscription_id: SubscriptionID) -> String {
        let pubsub_host = std::env::var("PUBSUB_HOST").ok().unwrap();
        let pubsub_callback_url =
            format!("{}{}/{}", pubsub_host, self.callback_path, subscription_id);

        pubsub_callback_url
    }
}

async fn default_handler(request: Request<Body>) -> StatusCode {
    println!("Default handler called {:?}", request);

    StatusCode::OK
}

#[derive(Debug, Deserialize)]
pub struct PubSubCallbackParams {
    #[serde(rename = "hub.challenge")]
    challenge: Option<String>,
    #[serde(rename = "hub.mode")]
    mode: Option<String>,
    #[serde(rename = "hub.topic")]
    topic: Option<String>,
    #[serde(rename = "hub.lease_seconds")]
    lease_seconds: Option<i64>,
}

async fn pubsub_challenge_handler(
    Query(params): Query<PubSubCallbackParams>,
) -> (StatusCode, String) {
    println!(
        "Received [{:?}] challenge for topic {:?}. Subscription lasts for [{:?}] seconds. Responding with challenge {:?}.",
        params.mode, params.topic, params.lease_seconds, params.challenge
    );

    (StatusCode::OK, params.challenge.unwrap_or_default())
}

async fn pubsub_callback_handler(
    Extension(subscriptions): Extension<Arc<Mutex<HashMap<SubscriptionID, Subscription>>>>,
    Path(subscription_id): Path<SubscriptionID>,
    payload: String,
) -> StatusCode {
    println!("Processing feed: {}", payload);

    {
        let handler = subscriptions
            .lock()
            .await
            .remove(&subscription_id)
            .unwrap()
            .handler;
        handler(payload).await;
    }

    StatusCode::OK
}
