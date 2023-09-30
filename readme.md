# gorillabot

A Discord bot for syncing the status (map and player count) of game server to a channel. It should support anything using the [A2S protocol](https://developer.valvesoftware.com/wiki/Server_queries) (including GoldSrc & Source Engine games), but it has only been tested with ARMA 3. The bot is written in Rust using [serenity](https://github.com/serenity-rs/serenity) for Discord integration and [a2s-rs](https://github.com/rumblefrog/a2s-rs) for querying the server. Subscriptions are stored in an embedded SQLite database.

The bot supports multiple subscriptions per channel, though currently you can only remove all subscriptions for a channel at once.

## Building

Because the bot uses `rusqlite` with the `bundled` feature which compiles a recent version of SQLite from source, you need a C compiler in your environment to build the bot (MSVC build tools, `build-essentials` or equivalent).

1. Install [Rust](https://www.rust-lang.org/tools/install) for your platform.
2. Clone the repository.
3. Run `cargo build --release` in the repository root. The binary will be located in `target/release/gorillabot`.
4. Copy the binary to a directory of your choice.
5. Configure the bot using environment variables (see below for reference).

## Configuration

The bot is configured using a few environment variables. The bot supports .env files, but you can also set the variables directly in your environment.

### `GORILLA_DISCORD_TOKEN`

The Discord bot token. See [Discord developer portal](https://discord.com/developers/docs/getting-started) for more information. You need to enable `Message Content Intent` permission for the bot to work.

### `GORILLA_POLL_INTERVAL_SECONDS`

The sleep time between each update of server statuses. Defaults to 30 seconds. On each update all servers are queried in series, so this is not a guarantee that the status of each server will be updated every 30 seconds.

## Usage

The bot can only be used by users who have the role `gorilladmin`. The role is not created automatically, so you need to add it manually to your server and give it to the users you want to be able to use the bot.

#### `!follow_server <server address>:<port>`

Parses & resolves the given server address and creates a new status subscription for the server. The address can be either a domain name or an IP address, and the port number is required (2303 for ARMA 3). The bot posts a message to the channel when the subscription is created, and then edits the message on each update.

### `!unfollow_server`

Removes all subscriptions from the channel.

## License

Licensed under the MIT license. See [license.md](license.md) for details.
