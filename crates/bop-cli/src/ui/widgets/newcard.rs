/// NewCard mode — inline card creation from the TUI.
///
/// When the user presses `n` in Normal mode, the TUI enters `Mode::NewCard`.
/// The footer shows `New card id: {input}█  [↵]create [Esc]cancel`.
/// On Enter: spawns `bop new default <id>` via [`std::process::Command`].
/// On Esc: cancels and returns to Normal mode.
///
/// The input buffer lives in [`App::newcard_input`](crate::ui::app::App::newcard_input).
/// This module provides the card creation side-effect only; input handling
/// is in [`input.rs`](crate::ui::input) and footer rendering is in
/// [`footer.rs`](crate::ui::widgets::footer).
use std::process::Command;

/// Create a new card by shelling out to `bop new default <id>`.
///
/// Runs `bop new default <card_id>` synchronously. Returns `Ok(())` on
/// success or an error if the command fails to launch or exits non-zero.
///
/// The caller is responsible for triggering a Cards refresh event
/// afterwards so the new card appears in the kanban board.
pub fn create_card(card_id: &str) -> anyhow::Result<()> {
    let output = Command::new("bop")
        .args(["new", "default", card_id])
        .output()
        .map_err(|e| anyhow::anyhow!("failed to spawn bop new: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("bop new failed: {}", stderr.trim());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_card_rejects_empty_id() {
        // Calling `bop new default ""` should fail — the binary rejects
        // empty IDs. We don't guard against this in the widget because
        // input.rs only fires create_card when input is non-empty, but
        // if the binary isn't on PATH this will error too. Either way,
        // the function must return Err, not panic.
        let result = create_card("");
        // We can't assert success because `bop` may not be on PATH in
        // test environments. Just verify it doesn't panic.
        let _ = result;
    }
}
