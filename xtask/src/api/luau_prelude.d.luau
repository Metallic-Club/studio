-- The hand-written part of api/metallic.d.luau: the data types, signals and globals the Luau backend
-- (crates/luau) builds itself instead of reading them from the class registry. `cargo xtask api-dump` copies this
-- file to the top of the definitions and generates the enums and classes after it.
--
-- KEEP IN SYNC with crates/luau/src/library.rs (globals, the `task` library, signals and connections) and
-- crates/luau/src/types.rs (the members of Vector3, Vector2, CFrame, Color3 and EnumItem). A member added there
-- goes here too; CI type-checks api/samples/ against the result with luau-lsp.

---------------------------------------------------------------------------------------------------------------------
-- Signals
---------------------------------------------------------------------------------------------------------------------

--- A connection made by `Connect` or `Once` (`typeof` says `RBXScriptConnection`).
declare extern type ScriptConnection with
    --- Whether the connection still delivers events.
    read Connected: boolean
    --- Stops delivering events to the listener.
    function Disconnect(self): ()
end

--- A signal of an instance, such as `part.Changed` (`typeof` says `RBXScriptSignal`). Listeners receive `T...`.
export type ScriptSignal<T... = ...any> = {
    --- Calls `listener` with the event's values every time the signal fires, each time in a new thread.
    Connect: (self: ScriptSignal<T...>, listener: (T...) -> ()) -> ScriptConnection,
    --- Like `Connect`, but only for the next event.
    Once: (self: ScriptSignal<T...>, listener: (T...) -> ()) -> ScriptConnection,
    --- Yields the running thread until the signal fires, then returns the event's values.
    Wait: (self: ScriptSignal<T...>) -> T...,
}

---------------------------------------------------------------------------------------------------------------------
-- Enums: the generated `Enum<Name>` item types extend `EnumItem`, and `Enum.<Name>` extends `Enum`.
---------------------------------------------------------------------------------------------------------------------

--- An item of an engine enum, such as `Enum.Material.Plastic`.
declare extern type EnumItem with
    --- The item's name, such as `"Plastic"`.
    read Name: string
    --- The item's number.
    read Value: number
    --- The name of the enum it belongs to, such as `"Material"`.
    read EnumType: string
end

--- An engine enum, such as `Enum.Material`.
declare extern type Enum with
    --- Every item of the enum, in order.
    function GetEnumItems(self): { EnumItem }
end

---------------------------------------------------------------------------------------------------------------------
-- Data types
---------------------------------------------------------------------------------------------------------------------

--- A 3D vector (Luau's native `vector`).
declare extern type Vector3 with
    read X: number
    read Y: number
    read Z: number
    read x: number
    read y: number
    read z: number
    --- The length.
    read Magnitude: number
    --- The vector scaled to length 1 (NaN components for the zero vector).
    read Unit: Vector3
    function Dot(self, other: Vector3): number
    function Cross(self, other: Vector3): Vector3
    function Lerp(self, goal: Vector3, alpha: number): Vector3
    function Min(self, ...: Vector3): Vector3
    function Max(self, ...: Vector3): Vector3
    function Abs(self): Vector3
    function Floor(self): Vector3
    function Ceil(self): Vector3
    function FuzzyEq(self, other: Vector3, epsilon: number?): boolean
    --- The angle to `other` in radians, signed around `axis` when one is given.
    function Angle(self, other: Vector3, axis: Vector3?): number
    function __add(self, other: Vector3): Vector3
    function __sub(self, other: Vector3): Vector3
    function __mul(self, other: Vector3): Vector3
    function __mul(self, other: number): Vector3
    function __div(self, other: Vector3): Vector3
    function __div(self, other: number): Vector3
    function __idiv(self, other: Vector3): Vector3
    function __idiv(self, other: number): Vector3
    function __unm(self): Vector3
end

declare Vector3: {
    new: (x: number?, y: number?, z: number?) -> Vector3,
    zero: Vector3,
    one: Vector3,
    xAxis: Vector3,
    yAxis: Vector3,
    zAxis: Vector3,
}

--- A 2D vector.
declare extern type Vector2 with
    read X: number
    read Y: number
    --- The length.
    read Magnitude: number
    --- The vector scaled to length 1.
    read Unit: Vector2
    function Dot(self, other: Vector2): number
    --- The 2D cross product (`x1 * y2 - y1 * x2`).
    function Cross(self, other: Vector2): number
    function Lerp(self, goal: Vector2, alpha: number): Vector2
    function Min(self, ...: Vector2): Vector2
    function Max(self, ...: Vector2): Vector2
    function Abs(self): Vector2
    function Floor(self): Vector2
    function Ceil(self): Vector2
    function FuzzyEq(self, other: Vector2, epsilon: number?): boolean
    function __add(self, other: Vector2): Vector2
    function __sub(self, other: Vector2): Vector2
    function __mul(self, other: Vector2): Vector2
    function __mul(self, other: number): Vector2
    function __div(self, other: Vector2): Vector2
    function __div(self, other: number): Vector2
    function __unm(self): Vector2
end

declare Vector2: {
    new: (x: number?, y: number?) -> Vector2,
    zero: Vector2,
    one: Vector2,
    xAxis: Vector2,
    yAxis: Vector2,
}

--- A position and a rotation.
declare extern type CFrame with
    read Position: Vector3
    read p: Vector3
    read X: number
    read Y: number
    read Z: number
    read LookVector: Vector3
    read RightVector: Vector3
    read UpVector: Vector3
    read XVector: Vector3
    read YVector: Vector3
    read ZVector: Vector3
    --- The rotation alone, at the origin.
    read Rotation: CFrame
    function Inverse(self): CFrame
    function Lerp(self, goal: CFrame, alpha: number): CFrame
    function ToOrientation(self): (number, number, number)
    function ToEulerAnglesXYZ(self): (number, number, number)
    function ToEulerAnglesYXZ(self): (number, number, number)
    --- The position, then the rotation matrix row by row.
    function GetComponents(self): (number, number, number, number, number, number, number, number, number, number, number, number)
    function components(self): (number, number, number, number, number, number, number, number, number, number, number, number)
    function ToWorldSpace(self, cf: CFrame): CFrame
    function ToObjectSpace(self, cf: CFrame): CFrame
    function PointToWorldSpace(self, point: Vector3): Vector3
    function PointToObjectSpace(self, point: Vector3): Vector3
    function VectorToWorldSpace(self, vector: Vector3): Vector3
    function VectorToObjectSpace(self, vector: Vector3): Vector3
    function FuzzyEq(self, other: CFrame, epsilon: number?): boolean
    function __mul(self, other: CFrame): CFrame
    function __mul(self, other: Vector3): Vector3
    function __add(self, other: Vector3): CFrame
    function __sub(self, other: Vector3): CFrame
end

declare CFrame: {
    new: (() -> CFrame)
        & ((position: Vector3) -> CFrame)
        & ((position: Vector3, lookAt: Vector3) -> CFrame)
        & ((x: number, y: number, z: number) -> CFrame)
        & ((x: number, y: number, z: number, qX: number, qY: number, qZ: number, qW: number) -> CFrame)
        & ((x: number, y: number, z: number, R00: number, R01: number, R02: number, R10: number, R11: number, R12: number, R20: number, R21: number, R22: number) -> CFrame),
    --- A rotation from angles in radians, applied Z, then Y, then X.
    Angles: (rx: number?, ry: number?, rz: number?) -> CFrame,
    fromEulerAnglesXYZ: (rx: number?, ry: number?, rz: number?) -> CFrame,
    --- A rotation from angles in radians, applied Z, then X, then Y.
    fromOrientation: (rx: number?, ry: number?, rz: number?) -> CFrame,
    fromEulerAnglesYXZ: (rx: number?, ry: number?, rz: number?) -> CFrame,
    lookAt: (at: Vector3, target: Vector3, up: Vector3?) -> CFrame,
    fromAxisAngle: (axis: Vector3, angle: number) -> CFrame,
    identity: CFrame,
}

--- A colour, each component from 0 to 1.
declare extern type Color3 with
    read R: number
    read G: number
    read B: number
    function Lerp(self, goal: Color3, alpha: number): Color3
    --- The colour as six hex digits, without `#`.
    function ToHex(self): string
    --- Hue, saturation and value, each from 0 to 1.
    function ToHSV(self): (number, number, number)
end

declare Color3: {
    new: (r: number?, g: number?, b: number?) -> Color3,
    fromRGB: (r: number?, g: number?, b: number?) -> Color3,
    fromHex: (hex: string) -> Color3,
    fromHSV: (h: number, s: number, v: number) -> Color3,
}

---------------------------------------------------------------------------------------------------------------------
-- Globals
---------------------------------------------------------------------------------------------------------------------

--- The game's root (`DataModel`).
declare game: DataModel
--- The game's `Workspace`.
declare workspace: Workspace
--- The script or module this code belongs to.
declare script: SourceContainer
--- A table every script of the VM shares.
declare shared: { [any]: any }

--- Writes a warning to the output.
declare function warn(...: any): ()
--- Seconds since the Unix epoch (the wall clock).
declare function tick(): number
--- Seconds of game time since the game started (the clock `task.wait` uses).
declare function time(): number
--- Runs a `ModuleScript` once and returns the value it returned, the same value for every caller.
declare function require(module: ModuleScript): any

--- The task scheduler.
declare task: {
    --- Runs `f` now, until it first yields.
    spawn: (f: ((...any) -> ...any) | thread, ...any) -> thread,
    --- Runs `f` at the end of the current resumption point.
    defer: (f: ((...any) -> ...any) | thread, ...any) -> thread,
    --- Runs `f` after `seconds` of game time.
    delay: (seconds: number?, f: ((...any) -> ...any) | thread, ...any) -> thread,
    --- Yields for at least `seconds` of game time (until the next Heartbeat without one); returns the time that
    --- passed.
    wait: (seconds: number?) -> number,
    --- Stops a thread that is not running; it never runs again.
    cancel: (thread: thread) -> (),
}
