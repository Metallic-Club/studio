//! `UserGameSettings` from Luau (#84): scripts read the player's graphics settings and cannot change them.

mod support;

use support::Game;

#[test]
fn scripts_read_the_graphics_settings_but_cannot_change_them() {
    let mut game = Game::new();
    game.run(
        "Settings",
        r##"
        local settings = game:GetService("UserGameSettings")
        print(settings.GraphicsPreset, settings.QualityLevel, settings.RenderScale, settings.VSync)
        print(settings.GraphicsPreset == Enum.GraphicsPreset.Auto)
        "##,
    );
    game.run(
        "Write",
        "local settings = game:GetService('UserGameSettings')\nsettings.QualityLevel = 4\n",
    );
    game.frame(1.0 / 60.0);
    assert_eq!(game.prints(), ["Enum.GraphicsPreset.Auto 2 1 true", "true"]);
    assert_eq!(
        game.errors(),
        ["Workspace.Write:2: Unable to assign property QualityLevel. Property is read only"]
    );
}
