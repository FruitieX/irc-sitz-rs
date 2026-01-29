irc-sitz-rs
===========

songleader for hosting a singalong "sitz" on IRC and/or Discord

very useful

features
--------

- `irc` - Enable IRC support (requires configuration)
- `discord` - Enable Discord bot support (requires configuration)

At least one feature must be enabled for the bot to be useful.

running
-------

```
cp Config.toml.example Config.toml
$EDITOR Config.toml

# Run with IRC support
cargo run --features irc

# Run with Discord support  
cargo run --features discord

# Run with both
cargo run --features irc,discord
```

Or using docker:

```
docker-compose up
```