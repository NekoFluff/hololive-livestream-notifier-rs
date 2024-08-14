#[derive(Debug)]

pub struct VideoMetadata {
    pub title: String,
    pub description: String,
    pub channel_id: String,
    pub channel_title: String,
    pub livestream_start_dt: chrono::DateTime<chrono::Utc>,
}

pub struct YoutubeClient {
    client: reqwest::Client,
}

impl YoutubeClient {
    pub fn new() -> Self {
        let client = reqwest::Client::new();
        Self { client }
    }

    pub async fn get_video_metadata(
        &self,
        video_id: &str,
    ) -> Result<VideoMetadata, Box<dyn std::error::Error>> {
        let url = "https://youtube.googleapis.com/youtube/v3/videos";
        let api_key = std::env::var("YOUTUBE_API_KEY")?;
        let params = [
            (
                "part",
                "snippet,contentDetails,statistics,liveStreamingDetails",
            ),
            ("id", video_id),
            ("key", &api_key),
        ];
        let response = self.client.get(url).query(&params).send().await?;
        let body = response.text().await?;
        println!("Body: {}", body);

        let json: serde_json::Value = serde_json::from_str(&body)?;
        let items = json["items"].as_array().ok_or("No items")?;
        let item = items.first().ok_or("No items")?;
        let snippet = item["snippet"].as_object().ok_or("No snippet")?;

        let title = snippet["title"].as_str().ok_or("No title")?;

        let description = snippet["description"].as_str().ok_or("No description")?;
        let channel_id = snippet["channelId"].as_str().ok_or("No channelId")?;
        let channel_title = snippet["channelTitle"].as_str().ok_or("No channelTitle")?;

        let live_streaming_details = item["liveStreamingDetails"]
            .as_object()
            .ok_or("No liveStreamingDetails")?;

        // e.g. 2022-03-04T18:12:32Z
        let livestream_start_dt = live_streaming_details["scheduledStartTime"]
            .as_str()
            .ok_or("No scheduledStartTime")?;

        Ok(VideoMetadata {
            title: title.to_string(),
            description: description.to_string(),
            channel_id: channel_id.to_string(),
            channel_title: channel_title.to_string(),
            // tags,
            livestream_start_dt: chrono::DateTime::parse_from_rfc3339(livestream_start_dt)?
                .to_utc(),
        })
    }
}
