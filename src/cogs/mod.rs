pub mod info;
pub mod lookup;
pub mod network;

/// Command groups are ordinary modules (a lightweight cog-like pattern); add a module here
/// and append its exported slash-command list to register another group.
pub fn commands() -> Vec<poise::Command<crate::Data, crate::Error>> {
    let mut commands = Vec::new();
    commands.extend(network::commands());
    commands.extend(lookup::commands());
    commands.extend(info::commands());
    commands
}
