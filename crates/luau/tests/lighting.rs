//! `Lighting`'s clock from Luau (#116): `ClockTime` wraps, and `TimeOfDay` and the minutes methods follow it.

mod support;

use support::Game;

#[test]
fn clock_time_wraps_instead_of_clamping() {
    let mut game = Game::new();
    game.run(
        "Wrap",
        r##"
        local Lighting = game:GetService("Lighting")
        Lighting.ClockTime = 25
        print("25 ->", Lighting.ClockTime)
        Lighting.ClockTime = -1
        print("-1 ->", Lighting.ClockTime)
        Lighting.ClockTime = 24
        print("24 ->", Lighting.ClockTime)
        -- The Roblox day/night loop, with no modulo.
        Lighting.ClockTime = 23.5
        Lighting.ClockTime += 1
        print("23.5 + 1 ->", Lighting.ClockTime)
        "##,
    );
    game.frame(1.0 / 60.0);
    game.assert_no_errors();
    assert_eq!(game.prints(), ["25 -> 1", "-1 -> 23", "24 -> 0", "23.5 + 1 -> 0.5"]);
}

#[test]
fn time_of_day_and_minutes_round_trip_with_clock_time() {
    let mut game = Game::new();
    game.run(
        "Clock",
        r##"
        local Lighting = game:GetService("Lighting")
        print(Lighting.TimeOfDay, Lighting:GetMinutesAfterMidnight())
        Lighting.ClockTime = 6.5
        print(Lighting.TimeOfDay, Lighting:GetMinutesAfterMidnight())
        Lighting.TimeOfDay = "21:15:00"
        print(Lighting.ClockTime, Lighting.TimeOfDay, Lighting:GetMinutesAfterMidnight())
        Lighting:SetMinutesAfterMidnight(90)
        print(Lighting.ClockTime, Lighting.TimeOfDay)
        Lighting:SetMinutesAfterMidnight(25 * 60)
        print(Lighting.ClockTime, Lighting.TimeOfDay)
        local ok, err = pcall(function() Lighting.TimeOfDay = "half past two" end)
        print(ok, string.find(err, "TimeOfDay must look like", 1, true) ~= nil, Lighting.ClockTime)
        "##,
    );
    game.frame(1.0 / 60.0);
    game.assert_no_errors();
    assert_eq!(
        game.prints(),
        [
            "14:00:00 840",
            "06:30:00 390",
            "21.25 21:15:00 1275",
            "1.5 01:30:00",
            "1 01:00:00",
            "false true 1",
        ]
    );
}

#[test]
fn time_of_day_signals_fire_when_clock_time_changes_and_the_sun_follows() {
    let mut game = Game::new();
    game.run(
        "Watch",
        r##"
        local Lighting = game:GetService("Lighting")
        Lighting:GetPropertyChangedSignal("TimeOfDay"):Connect(function() print("time of day", Lighting.TimeOfDay) end)
        Lighting:GetPropertyChangedSignal("ClockTime"):Connect(function() print("clock time", Lighting.ClockTime) end)
        Lighting.ClockTime = 6
        print("sun", Lighting.SunDirection.X > 0.99, Lighting:GetSunDirection().X > 0.99)
        "##,
    );
    // Signals are deferred to the end of the frame, so each change gets a frame of its own.
    game.frame(1.0 / 60.0);
    game.run(
        "Evening",
        r##"
        local Lighting = game:GetService("Lighting")
        Lighting.TimeOfDay = "18:00:00"
        print("sun", Lighting.SunDirection.X < -0.99)
        "##,
    );
    game.frame(1.0 / 60.0);
    game.assert_no_errors();
    let prints = game.prints();
    for expected in [
        "sun true true",
        "clock time 6",
        "time of day 06:00:00",
        "sun true",
        "clock time 18",
        "time of day 18:00:00",
    ] {
        assert!(
            prints.iter().any(|p| p == expected),
            "missing {expected:?} in {prints:?}"
        );
    }
    // Each change fired each signal once.
    assert_eq!(
        prints.iter().filter(|p| p.starts_with("time of day")).count(),
        2,
        "{prints:?}"
    );
    assert_eq!(
        prints.iter().filter(|p| p.starts_with("clock time")).count(),
        2,
        "{prints:?}"
    );
}
