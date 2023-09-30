use serenity::model::prelude::{ChannelId, GuildId, MessageId};

#[derive(Debug)]
pub struct Subscription {
    pub id: Option<i64>,
    pub guild_id: GuildId,
    pub channel_id: ChannelId,
    pub message_id: MessageId,
    pub server_hostname: String,
}
