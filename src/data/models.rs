use mongodb::bson::doc;
use mongodb::bson::DateTime;
use serde::Deserializer;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(try_from = "&str")]

pub struct Username(String);

impl<'a> TryFrom<&'a str> for Username {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value.len() < 3 {
            return Err(format!("Username ({value}) is too short").to_string());
        }
        Ok(Self(value.to_string()))
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(try_from = "&str")]
pub struct Email(String);

impl<'a> TryFrom<&'a str> for Email {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if !value.contains('@') {
            return Err(format!("Email ({value}) is invalid").to_string());
        }
        Ok(Self(value.to_string()))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Livestream {
    pub author: String,
    pub url: String,
    pub date: DateTime,
    pub title: String,
    pub updated: DateTime,
}

#[derive(Debug, Deserialize)]
pub struct Feed {
    #[serde(rename = "firstName")]
    pub first_name: String,
    #[serde(rename = "lastName")]
    pub last_name: String,
    #[serde(rename = "topicURL", deserialize_with = "de_url")]
    pub topic_url: reqwest::Url,
    pub group: String,
    #[serde(default)]
    pub generation: u8,
}

fn de_url<'de, D>(deserializer: D) -> Result<reqwest::Url, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    let url = reqwest::Url::parse(&s).map_err(serde::de::Error::custom)?;
    Ok(url)
}
