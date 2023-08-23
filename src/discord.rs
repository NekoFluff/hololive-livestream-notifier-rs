use poise::serenity_prelude::{self as serenity, ChannelId};

pub async fn send_message_to_channel(
    channel_name: &str,
    message: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let http = serenity::http::client::Http::new(&std::env::var("DISCORD_TOKEN")?);

    let channels = get_channels(channel_name).await?;

    for channel in channels {
        ChannelId(channel.id.0)
            .send_message(&http, |m| m.content(message))
            .await?;
    }

    Ok(())
}

pub async fn send_message_to_user(
    user_id: serenity::UserId,
    message: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let http = serenity::http::client::Http::new(&std::env::var("DISCORD_TOKEN")?);

    user_id
        .create_dm_channel(&http)
        .await?
        .send_message(&http, |m| m.content(message))
        .await?;

    Ok(())
}

async fn get_channels(
    channel_name: &str,
) -> Result<Vec<serenity::model::channel::GuildChannel>, Box<dyn std::error::Error>> {
    let http = serenity::http::client::Http::new(&std::env::var("DISCORD_TOKEN")?);

    let guilds = http.get_guilds(None, None).await?;

    let mut channels = Vec::new();

    for guild in guilds {
        let mut guild_channels = http
            .get_channels(guild.id.0)
            .await?
            .into_iter()
            .filter(|channel| channel.name == channel_name)
            .collect::<Vec<_>>();

        channels.append(&mut guild_channels);
    }

    Ok(channels)
}
