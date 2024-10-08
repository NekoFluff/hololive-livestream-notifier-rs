mod commands;
mod cron;
mod data;
mod discord;
mod youtube;
use hololive_livestream_notifier_rs::pubsub;

use axum::{
    body::Body,
    extract::Query,
    http::Request,
    http::StatusCode,
    routing::{get, post},
    Extension, Router,
};
use chrono::{DateTime, Datelike, FixedOffset, NaiveDateTime, Timelike, Utc};
use cron::LivestreamScheduler;
use dotenv::dotenv;
use poise::serenity_prelude::{self as serenity};
use quick_xml::de::from_str;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use std::{env::var, time::Duration};
use tokio::sync::Mutex;

use crate::discord::send_message_to_user;

// Types used by all command functions
type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

// Custom data passed to all command functions
pub struct Data {}

// TODO LIST
// TODO: Add logging package
// TODO: Host on heroku
// TODO: Add tracing
// TODO: Reorganize code

#[derive(Debug, Deserialize)]
pub struct YTCallbackParams {
    #[serde(rename = "hub.challenge")]
    challenge: Option<String>,
    #[serde(rename = "hub.mode")]
    mode: Option<String>,
    #[serde(rename = "hub.topic")]
    topic: Option<String>,
    #[serde(rename = "hub.lease_seconds")]
    lease_seconds: Option<i64>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    // --- Start of temp code
    // let temp_livestream_scheduler = Arc::new(Mutex::new(LivestreamScheduler::new().await));

    // process_url(
    //     Arc::clone(&temp_livestream_scheduler),
    //     "https://www.youtube.com/watch?v=I9zWxlP1VAM",
    //     0,
    // )
    // .await
    // .unwrap();

    // abort();
    // --- End of temp code

    tokio::spawn(start_bot());
    let livestream_scheduler = Arc::new(Mutex::new(LivestreamScheduler::new().await));

    // tracing_subscriber::fmt::init();
    setup_existing_livestream_notifications(Arc::clone(&livestream_scheduler)).await;
    tokio::spawn(subscribe_to_feeds());

    let app = Router::new()
        .route("/", get(default_handler))
        .route("/yt-pubsub", get(yt_pubsub_challenge_handler))
        .route("/yt-pubsub", post(yt_pubsub_callback))
        .layer(Extension(livestream_scheduler));
    let addr = SocketAddr::from(([0, 0, 0, 0], std::env::var("PORT")?.parse()?));

    // tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();

    Ok(())
}

async fn on_error(error: poise::FrameworkError<'_, Data, Error>) {
    match error {
        poise::FrameworkError::Setup { error, .. } => panic!("Failed to start bot: {:?}", error),
        poise::FrameworkError::Command { error, ctx } => {
            println!("Error in command `{}`: {:?}", ctx.command().name, error,);
        }
        error => {
            if let Err(e) = poise::builtins::on_error(error).await {
                println!("Error while handling error: {}", e)
            }
        }
    }
}

async fn subscribe_to_feeds() {
    let feeds = data::Mongo::new().await.get_feeds().await.unwrap();
    let pubsub_callback_url = std::env::var("PUBSUB_CALLBACK_URL").ok().unwrap();
    let pubsub_callback_url = reqwest::Url::parse(&pubsub_callback_url).ok().unwrap();
    let mut pubsub = pubsub::PubSub::new(pubsub_callback_url);

    for feed in feeds {
        if let Err(e) = pubsub.subscribe(feed.topic_url.clone()).await {
            println!("Error subscribing to {:?}: {:?}", feed.topic_url, e);
            continue;
        }

        println!(
            "Sent subscription request for {:?} {:?} ({:?})",
            feed.first_name,
            feed.last_name,
            feed.topic_url.as_str()
        );
    }
}

async fn setup_existing_livestream_notifications(
    livestream_scheduler: Arc<Mutex<LivestreamScheduler>>,
) {
    let mongo = data::Mongo::new().await;
    let livestreams = mongo.get_livestreams().await.unwrap();

    for livestream in livestreams {
        setup_livestream_notifications(Arc::clone(&livestream_scheduler), livestream)
            .await
            .unwrap();
    }
}

async fn start_bot() {
    let options = poise::FrameworkOptions {
        commands: vec![
            commands::help(),
            commands::ping(),
            // commands::vote(),
            // commands::getvotes(),
        ],
        prefix_options: poise::PrefixFrameworkOptions {
            prefix: Some("~".into()),
            edit_tracker: Some(poise::EditTracker::for_timespan(Duration::from_secs(3600))),
            additional_prefixes: vec![
                poise::Prefix::Literal("hey bot"),
                poise::Prefix::Literal("hey bot,"),
            ],
            ..Default::default()
        },
        on_error: |error| Box::pin(on_error(error)),
        pre_command: |ctx| {
            Box::pin(async move {
                println!("Executing command {}...", ctx.command().qualified_name);
            })
        },
        post_command: |ctx| {
            Box::pin(async move {
                println!("Executed command {}!", ctx.command().qualified_name);
            })
        },
        command_check: Some(|ctx| {
            Box::pin(async move {
                if ctx.author().id == 123456789 {
                    return Ok(false);
                }
                Ok(true)
            })
        }),
        skip_checks_for_owners: false,
        event_handler: |_ctx, event, _framework, _data| {
            Box::pin(async move {
                println!("Got an event in event handler: {:?}", event.name());
                Ok(())
            })
        },
        ..Default::default()
    };

    poise::Framework::builder()
        .token(var("DISCORD_TOKEN").expect("Missing `DISCORD_TOKEN` env var"))
        .setup(move |ctx, _ready, framework| {
            Box::pin(async move {
                println!("Logged in as {}", _ready.user.name);
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                Ok(Data {
                    // votes: Mutex::new(HashMap::new()),
                })
            })
        })
        .options(options)
        .intents(serenity::GatewayIntents::non_privileged())
        .run()
        .await
        .expect("Failed to start bot");
}

async fn default_handler(request: Request<Body>) -> StatusCode {
    println!("Default handler called {:?}", request);

    StatusCode::OK
}

async fn yt_pubsub_challenge_handler(
    Query(params): Query<YTCallbackParams>,
) -> (StatusCode, String) {
    println!(
        "Received [{:?}] challenge for topic {:?}. Subscription lasts for [{:?}] seconds. Responding with challenge {:?}.",
        params.mode, params.topic, params.lease_seconds, params.challenge
    );

    (StatusCode::OK, params.challenge.unwrap_or_default())
}

async fn yt_pubsub_callback(
    Extension(livestream_scheduler): Extension<Arc<Mutex<cron::LivestreamScheduler>>>,
    payload: String,
) -> StatusCode {
    println!("Processing youtube feed: {}", payload);

    let yt_feed = from_str::<pubsub::YoutubeFeed>(&payload).unwrap();

    println!("Parsed feed: {:?}", yt_feed);

    {
        let yt_feed_json_str = serde_json::to_string_pretty(&yt_feed).unwrap().to_string();
        let yt_feed_json_str = format!("Processing feed:\n```json\n{}\n```", yt_feed_json_str);
        tokio::spawn(send_message_to_developer(yt_feed_json_str));
    }

    let livestream_url = yt_feed.entry.first().unwrap().link.href.as_str();

    process_url(
        livestream_scheduler,
        livestream_url,
        yt_feed
            .entry
            .first()
            .unwrap()
            .updated
            .unwrap()
            .timestamp_millis(),
    )
    .await
    .unwrap();

    StatusCode::OK
}

async fn process_url(
    livestream_scheduler: Arc<Mutex<LivestreamScheduler>>,
    livestream_url: &str,
    updated_ts_ms: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let video_id = if let Some(captures) = regex::Regex::new(r"v=([^&]+)")
        .unwrap()
        .captures(livestream_url)
    {
        captures.get(1).map_or("", |m| m.as_str())
    } else {
        ""
    };

    let mongo = data::Mongo::new().await;
    let livestream = mongo.get_livestream(livestream_url).await;

    if livestream.is_err() {
        println!("Error getting livestream: {}", livestream.err().unwrap());
        return Ok(());
    }

    let livestream = livestream.unwrap();
    let data = youtube::YoutubeClient::new()
        .get_video_metadata(video_id)
        .await
        .unwrap();

    // if stream_dt.is_err() {
    //     tokio::spawn(send_message_to_developer(format!(
    //         "[{}] Error getting stream datetime: {}",
    //         livestream_url,
    //         stream_dt.err().unwrap()
    //     )));
    //     return StatusCode::OK;
    // }

    // let stream_dt = stream_dt.unwrap();

    let stream_dt = data.livestream_start_dt;

    if stream_dt < Utc::now() {
        println!("Stream already started ({})", livestream_url);
        return Ok(());
    }

    let stream_ts_ms = stream_dt.timestamp_millis();
    println!("Stream start datetime: {:?}", stream_dt);

    match livestream {
        Some(mut livestream) => {
            let current_stream_ts_ms = livestream.date.timestamp_millis();

            if current_stream_ts_ms != stream_ts_ms {
                livestream.title = data.title;
                livestream.author = data.channel_title;
                livestream.date = mongodb::bson::DateTime::from_millis(stream_ts_ms);
                livestream.updated = mongodb::bson::DateTime::from_millis(updated_ts_ms);

                mongo.upsert_livestream(&livestream).await.unwrap();
                send_will_livestream_message(&livestream).await.unwrap();
                setup_livestream_notifications(Arc::clone(&livestream_scheduler), livestream)
                    .await
                    .unwrap();
            }
        }
        None => {
            let livestream = data::models::Livestream {
                title: data.title,
                author: data.channel_title,
                url: livestream_url.to_string(),
                date: mongodb::bson::DateTime::from_millis(stream_ts_ms),
                updated: mongodb::bson::DateTime::from_millis(updated_ts_ms),
            };
            mongo.insert_livestream(&livestream).await.unwrap();
            send_will_livestream_message(&livestream).await.unwrap();
            setup_livestream_notifications(Arc::clone(&livestream_scheduler), livestream)
                .await
                .unwrap();
        }
    }

    tokio::spawn(send_message_to_developer(format!(
        "Processed livestream: {}",
        livestream_url
    )));

    Ok(())
}

pub async fn setup_livestream_notifications(
    livestream_scheduler: Arc<Mutex<LivestreamScheduler>>,
    livestream: data::models::Livestream,
) -> Result<(), Box<dyn std::error::Error>> {
    let timestamp = livestream.date.timestamp_millis() / 1000;
    let date = DateTime::<Utc>::from_utc(
        NaiveDateTime::from_timestamp_opt(timestamp, 0).unwrap(),
        Utc,
    );

    let cron_schedule_str = format!(
        "{} {} {} {} {} *",
        date.second(),
        date.minute(),
        date.hour(),
        date.day(),
        date.month()
    );

    let stream_url = livestream.url.clone();
    let livestream2 = livestream.clone();
    livestream_scheduler
        .lock()
        .await
        .schedule_livestream_notification(
            stream_url.as_str(),
            &cron_schedule_str,
            Box::new(move |_job_uuid, _scheduler| {
                let livestream = livestream.clone();
                Box::pin(async move {
                    send_is_live_message(&livestream).await.unwrap();
                })
            }),
        )
        .await
        .unwrap();

    let date = date - chrono::Duration::minutes(15);
    let cron_reminder_schedule_str = format!(
        "{} {} {} {} {} *",
        date.second(),
        date.minute(),
        date.hour(),
        date.day(),
        date.month()
    );
    livestream_scheduler
        .lock()
        .await
        .schedule_livestream_notification(
            format!("{}-reminder", stream_url).as_str(),
            &cron_reminder_schedule_str,
            Box::new(move |_job_uuid, _scheduler| {
                let livestream = livestream2.clone();
                Box::pin(async move {
                    send_livestream_reminder(&livestream).await.unwrap();
                })
            }),
        )
        .await
        .unwrap();

    Ok(())
}

pub async fn send_will_livestream_message(
    livestream: &data::models::Livestream,
) -> Result<(), Box<dyn std::error::Error>> {
    let timestamp = livestream.date.timestamp_millis() / 1000;
    let mst_dt = DateTime::<FixedOffset>::from_utc(
        NaiveDateTime::from_timestamp_opt(timestamp, 0).unwrap(),
        FixedOffset::west_opt(7 * 3600).unwrap(),
    );

    let message = format!(
        "[{}] will livestream on [{}] - [{}]",
        livestream.author,
        mst_dt.format("%a, %b %e, %l:%M %p UTC%z"),
        livestream.url
    );

    discord::send_message_to_channel("hololive-notifications", &message).await?;

    Ok(())
}

pub async fn send_livestream_reminder(
    livestream: &data::models::Livestream,
) -> Result<(), Box<dyn std::error::Error>> {
    let message = format!(
        "[{}] Livestream starting in 15 minutes! - [{}]",
        livestream.author, livestream.url
    );

    discord::send_message_to_channel("hololive-notifications", &message).await?;

    Ok(())
}

pub async fn send_is_live_message(
    livestream: &data::models::Livestream,
) -> Result<(), Box<dyn std::error::Error>> {
    let message = format!(
        "[{}] Livestream starting! {}",
        livestream.author, livestream.url
    );

    discord::send_message_to_channel("hololive-stream-started", &message).await?;

    Ok(())
}

pub async fn send_message_to_developer(message: String) {
    if let Ok(developer_id) = var("DEVELOPER_USER_ID") {
        let user_id = serenity::UserId(developer_id.parse::<u64>().unwrap());
        send_message_to_user(user_id, &message).await.unwrap();
    }
}
