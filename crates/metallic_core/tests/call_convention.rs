//! The method calling convention, end to end through a test `HostApi` (#461).
//!
//! `World` instances come from the real registry. The fixture registry below declares classes with
//! the same names, so its methods run on real instances without changing the real registry.
use metallic_core::classes::{
    self, ApiError, ApiErrorKind, CallContext, CallOutcome, ClassDesc, ClassFlags, ClassId,
    EnumItemRef, FunctionRef, FunctionSig, Handle, HostApi, HostCall, HostMethodError, HostMethods,
    MemberCategory, MemberId, MemberKind, MethodCall, MethodDesc, MethodDispatch, MethodImpl,
    Param, Registry, ScriptValue, Side, SignalKey, Ty, WaitRequest, member_of,
};
use metallic_core::{
    CFrame, Color3, ConnectionId, CoreError, InstanceId, UDim2, Value, Vec3, World,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::LazyLock;

const CONTEXT: CallContext = CallContext {
    side: Side::Server,
    script: None,
    local_player: None,
};

/// What the host-implemented fixture methods read.
#[derive(Default)]
struct State {
    calls: u32,
}

/// Decodes one argument of every kind and returns them.
fn all_kinds(call: &mut MethodCall) -> Result<CallOutcome, ApiError> {
    let array = call.array(10)?.to_vec();
    let map = call.map(11)?.to_vec();
    Ok(ScriptValue::Tuple(vec![
        call.bool(0)?.into(),
        ScriptValue::Int(call.int(1)?),
        call.number(2)?.into(),
        call.string(3)?.into(),
        call.instance(4)?.into(),
        ScriptValue::Vector3(call.vector3(5)?),
        call.cframe(6)?.into(),
        ScriptValue::Color3(call.color3(7)?),
        ScriptValue::UDim2(call.udim2(8)?),
        ScriptValue::EnumItem(call.enum_item(9)?),
        ScriptValue::Array(array),
        ScriptValue::Map(map),
        ScriptValue::Function(call.function(12)?),
    ])
    .into())
}

fn optional(call: &mut MethodCall) -> Result<CallOutcome, ApiError> {
    let first = call.opt_number(0)?;
    let second = call.opt_string(1)?;
    Ok(ScriptValue::Tuple(vec![
        first.unwrap_or(-1.0).into(),
        second.unwrap_or("left out").into(),
    ])
    .into())
}

fn class_name_of_call(call: &mut MethodCall) -> Result<CallOutcome, ApiError> {
    Ok(call.class_name().into())
}

fn count_arguments(call: &mut MethodCall) -> Result<CallOutcome, ApiError> {
    Ok(ScriptValue::Int(call.args.len() as i64).into())
}

/// Keeps its own text for a wrong or missing argument, as a migrated method does.
fn manual(call: &mut MethodCall) -> Result<CallOutcome, ApiError> {
    let target = call
        .arg(0)
        .as_cframe()
        .ok_or_else(|| ApiError::message("Manual: expected a CFrame"))?;
    Ok(target.into())
}

fn wait_for_child(call: &mut MethodCall) -> Result<CallOutcome, ApiError> {
    Ok(CallOutcome::Wait(WaitRequest::Child {
        parent: call.this,
        name: call.string(0)?.into(),
        timeout: call.opt_number(1)?,
    }))
}

fn waits_undeclared(_: &mut MethodCall) -> Result<CallOutcome, ApiError> {
    Ok(CallOutcome::Wait(WaitRequest::Seconds(1.0)))
}

/// A world error raised with `?`: the host fills in class and member, the text stays the world's.
fn world_refuses(call: &mut MethodCall) -> Result<CallOutcome, ApiError> {
    call.world.set(call.this, "NoSuchProperty", Value::Nil)?;
    Ok(ScriptValue::Nil.into())
}

fn base_version(_: &mut MethodCall) -> Result<CallOutcome, ApiError> {
    Ok("base".into())
}

fn sub_version(_: &mut MethodCall) -> Result<CallOutcome, ApiError> {
    Ok("sub".into())
}

fn host_body(call: &mut HostCall<'_, State>) -> Result<CallOutcome, ApiError> {
    call.state.calls += 1;
    Ok(call.call.number(0)?.into())
}

fn get_pivot(call: &mut HostCall<'_, State>) -> Result<CallOutcome, ApiError> {
    let call = &mut call.call;
    Ok(metallic_core::transform::get_pivot(call.world, call.this)?.into())
}

static ANY_FUNCTION: FunctionSig = FunctionSig {
    params: &[],
    returns: &Ty::Any,
};

/// One method per case. `World` makes the instances; these only have to match by class name.
static FIXTURE_INSTANCE: ClassDesc = ClassDesc::new("Instance", None).methods(&[
    MethodDesc::native("AllKinds", all_kinds).params(&[
        Param::new("a", Ty::Bool),
        Param::new("b", Ty::Int),
        Param::new("c", Ty::Number),
        Param::new("d", Ty::String),
        Param::new("e", Ty::Instance(None)),
        Param::new("f", Ty::Vector3),
        Param::new("g", Ty::CFrame),
        Param::new("h", Ty::Color3),
        Param::new("i", Ty::UDim2),
        Param::new("j", Ty::Enum("Material")),
        Param::new("k", Ty::Array(&Ty::Number)),
        Param::new("l", Ty::Map(&Ty::Bool)),
        Param::new("m", Ty::Function(&ANY_FUNCTION)),
    ]),
    MethodDesc::native("Optional", optional).params(&[
        Param::new("first", Ty::Number).optional(),
        Param::new("second", Ty::String).optional(),
    ]),
    MethodDesc::native("Required", base_version).params(&[
        Param::new("amount", Ty::Number),
        Param::new("face", Ty::Enum("NormalId")),
        Param::new("rest", Ty::String).optional(),
    ]),
    MethodDesc::native("OneFolder", class_name_of_call)
        .params(&[Param::new("folder", Ty::Instance(Some("Folder")))]),
    MethodDesc::native("MaybePart", class_name_of_call).params(&[Param::new(
        "part",
        Ty::Optional(&Ty::Instance(Some("BasePart"))),
    )]),
    MethodDesc::native("Whole", class_name_of_call).params(&[Param::new("n", Ty::Int)]),
    MethodDesc::native("Numbers", count_arguments)
        .params(&[Param::new("first", Ty::String)])
        .variadic(Ty::Number),
    MethodDesc::native("Manual", manual).params(&[Param::new("target", Ty::CFrame).manual()]),
    MethodDesc::native("WaitForIt", wait_for_child)
        .params(&[
            Param::new("name", Ty::String),
            Param::new("timeout", Ty::Number).optional(),
        ])
        .yields(),
    MethodDesc::native("WaitsUndeclared", waits_undeclared),
    MethodDesc::native("WorldRefuses", world_refuses),
    MethodDesc::native("WhereAmI", class_name_of_call),
    MethodDesc::native("Version", base_version),
    MethodDesc::native("HiddenBelow", base_version),
    MethodDesc::host("HostMethod").params(&[Param::new("x", Ty::Number)]),
    MethodDesc::host("Unimplemented"),
    MethodDesc {
        implementation: MethodImpl::Luau,
        ..MethodDesc::host("InLuau")
    },
]);

static FIXTURE_FOLDER: ClassDesc = ClassDesc::new("Folder", Some("Instance"))
    .methods(&[MethodDesc::native("Version", sub_version).overrides()])
    .hide(&["HiddenBelow"]);
static FIXTURE_PART: ClassDesc = ClassDesc::new("Part", Some("Instance"));

static FIXTURE: LazyLock<Registry> = LazyLock::new(|| {
    classes::resolve(&[&FIXTURE_INSTANCE, &FIXTURE_FOLDER, &FIXTURE_PART])
        .expect("the fixture classes resolve")
});

/// The test `HostApi`: the shape every language binding has, minus the language.
struct TestHost {
    registry: &'static Registry,
    methods: HostMethods<State>,
    state: State,
    world: World,
    context: CallContext,
}

impl TestHost {
    fn fixture() -> TestHost {
        let mut methods = HostMethods::new(&FIXTURE);
        methods
            .implement("Instance", "HostMethod", host_body)
            .unwrap();
        TestHost::with(&FIXTURE, methods)
    }

    fn real() -> TestHost {
        let mut methods = HostMethods::new(classes::registry());
        methods
            .implement("PVInstance", "GetPivot", get_pivot)
            .unwrap();
        TestHost::with(classes::registry(), methods)
    }

    fn with(registry: &'static Registry, methods: HostMethods<State>) -> TestHost {
        TestHost {
            registry,
            methods,
            state: State::default(),
            world: World::new_with_services(),
            context: CONTEXT,
        }
    }

    fn create(&mut self, class: &str) -> InstanceId {
        let parent = self.world.root("Workspace");
        self.world.create(class, parent).unwrap()
    }

    /// As a generated wrapper would: the declaring class by name, then the member by name.
    fn call(
        &mut self,
        h: Handle,
        class: &str,
        method: &str,
        args: &[ScriptValue],
    ) -> Result<CallOutcome, ApiError> {
        let class = self.resolve_class(class).unwrap();
        let member = self.resolve_member(class, method).unwrap();
        self.call_method(h, class, member, args)
    }

    fn ret(&mut self, h: Handle, method: &str, args: &[ScriptValue]) -> ScriptValue {
        match self.call(h, "Instance", method, args).unwrap() {
            CallOutcome::Return(value) => value,
            CallOutcome::Wait(wait) => panic!("{method} waited: {wait:?}"),
        }
    }

    fn err(&mut self, h: Handle, method: &str, args: &[ScriptValue]) -> ApiError {
        self.call(h, "Instance", method, args).unwrap_err()
    }

    fn text(&mut self, h: Handle, method: &str, args: &[ScriptValue]) -> String {
        self.err(h, method, args).to_string()
    }
}

impl HostApi for TestHost {
    fn resolve_class(&self, name: &str) -> Option<ClassId> {
        self.registry.class_id(name)
    }

    fn resolve_member(&self, class: ClassId, name: &str) -> Option<MemberId> {
        self.registry.class(class)?.member(name)
    }

    fn class_of(&self, h: Handle) -> Result<ClassId, ApiError> {
        self.world
            .class_name(h)
            .and_then(|name| self.registry.class_id(name))
            .ok_or_else(|| {
                let name = classes::UNNAMED_INSTANCE.to_owned();
                ApiError::new("", "", ApiErrorKind::Destroyed { name })
            })
    }

    fn is_a(&self, h: Handle, class: ClassId) -> Result<bool, ApiError> {
        let own = self.class_of(h)?;
        Ok(self.registry.class(own).is_some_and(|c| c.is_a(class)))
    }

    fn get_property(
        &mut self,
        h: Handle,
        class: ClassId,
        m: MemberId,
    ) -> Result<ScriptValue, ApiError> {
        let (_, member) = member_of(self.registry, &self.world, h, class, m)?;
        let MemberKind::Property { desc, .. } = member.kind else {
            return Err(ApiError::message("not a property"));
        };
        Ok(self.world.get(h, desc.name)?.into())
    }

    fn set_property(
        &mut self,
        h: Handle,
        class: ClassId,
        m: MemberId,
        v: ScriptValue,
    ) -> Result<(), ApiError> {
        let (own, member) = member_of(self.registry, &self.world, h, class, m)?;
        let MemberKind::Property { desc, .. } = member.kind else {
            return Err(ApiError::message("not a property"));
        };
        if desc.readonly {
            return Err(ApiError::new(own.name(), desc.name, ApiErrorKind::ReadOnly));
        }
        let value = v.to_value(desc.ty).ok_or_else(|| {
            ApiError::from(CoreError::TypeMismatch {
                property: desc.name.to_owned(),
                expected: desc.ty.name().to_owned(),
                found: v.type_name(&self.world).to_string(),
            })
        })?;
        self.world.set(h, desc.name, value)?;
        Ok(())
    }

    fn call_method(
        &mut self,
        h: Handle,
        class: ClassId,
        m: MemberId,
        args: &[ScriptValue],
    ) -> Result<CallOutcome, ApiError> {
        MethodDispatch {
            registry: self.registry,
            methods: &self.methods,
            world: &mut self.world,
            state: &mut self.state,
            context: &self.context,
        }
        .call(h, class, m, args)
    }

    fn connect_signal(
        &mut self,
        _: Handle,
        _: ClassId,
        _: MemberId,
        _: u32,
        _: FunctionRef,
    ) -> Result<ConnectionId, ApiError> {
        Err(ApiError::message("signals arrive with #463"))
    }

    fn disconnect(&mut self, _: ConnectionId) {}

    fn set_callback(
        &mut self,
        _: Handle,
        _: ClassId,
        _: MemberId,
        _: Option<FunctionRef>,
    ) -> Result<(), ApiError> {
        Err(ApiError::message("callbacks arrive with #463"))
    }

    fn new_instance(&mut self, class: ClassId, parent: Option<Handle>) -> Result<Handle, ApiError> {
        let desc = self
            .registry
            .class(class)
            .ok_or_else(|| ApiError::message("no such class"))?
            .desc;
        if !desc.flags.contains(ClassFlags::CREATABLE) {
            let kind = ApiErrorKind::NotCreatable {
                class: desc.name.into(),
            };
            return Err(ApiError::new(desc.name, "", kind));
        }
        Ok(self.world.create(desc.name, parent)?)
    }
}

fn string(text: &str) -> ScriptValue {
    text.into()
}

#[test]
fn every_script_value_kind_decodes_through_its_accessor() {
    let mut host = TestHost::fixture();
    let this = host.create("Folder");
    let other = host.create("Part");
    let cframe = CFrame::from_position(Vec3::new(1.0, 2.0, 3.0));
    let item = EnumItemRef::new("Material", "Wood");
    let args = [
        ScriptValue::Bool(true),
        ScriptValue::Int(-4),
        ScriptValue::Number(2.5),
        string("text"),
        ScriptValue::Instance(other),
        ScriptValue::Vector3(Vec3::new(4.0, 5.0, 6.0)),
        cframe.into(),
        ScriptValue::Color3(Color3::new(7.0, 8.0, 9.0)),
        ScriptValue::UDim2(UDim2::new(0.5, 1.0, 0.25, 2.0)),
        ScriptValue::EnumItem(item),
        ScriptValue::Array(vec![ScriptValue::Number(1.0), ScriptValue::Int(2)]),
        ScriptValue::Map(vec![(string("key"), ScriptValue::Bool(false))]),
        ScriptValue::Function(FunctionRef(7)),
    ];
    assert_eq!(
        host.ret(this, "AllKinds", &args),
        ScriptValue::Tuple(args.to_vec())
    );
}

#[test]
fn an_int_is_a_number_and_a_whole_number_is_an_int() {
    let mut host = TestHost::fixture();
    let this = host.create("Folder");
    assert_eq!(
        host.ret(this, "Optional", &[ScriptValue::Int(3)]),
        ScriptValue::Tuple(vec![3.0.into(), "left out".into()])
    );
    for whole in [3.0, -3.0, -9_223_372_036_854_775_808.0] {
        host.ret(this, "Whole", &[ScriptValue::Number(whole)]);
    }
    for not_whole in [3.5, 9_223_372_036_854_775_808.0, f64::NAN, f64::INFINITY] {
        assert_eq!(
            host.text(this, "Whole", &[ScriptValue::Number(not_whole)]),
            "Folder:Whole: argument #1 expected integer, got number"
        );
    }
}

#[test]
fn optional_arguments_may_be_left_out_or_nil() {
    let mut host = TestHost::fixture();
    let this = host.create("Folder");
    let tuple = |first: f64, second: &str| ScriptValue::Tuple(vec![first.into(), second.into()]);
    assert_eq!(host.ret(this, "Optional", &[]), tuple(-1.0, "left out"));
    assert_eq!(
        host.ret(this, "Optional", &[ScriptValue::Number(1.0)]),
        tuple(1.0, "left out")
    );
    assert_eq!(
        host.ret(this, "Optional", &[ScriptValue::Nil, string("given")]),
        tuple(-1.0, "given")
    );
    assert_eq!(
        host.ret(this, "Optional", &[ScriptValue::Nil, ScriptValue::Nil]),
        tuple(-1.0, "left out")
    );
    // Given, but wrong, is still wrong.
    assert_eq!(
        host.text(this, "Optional", &[string("no")]),
        "Folder:Optional: argument #1 expected number, got string"
    );
}

#[test]
fn missing_and_wrong_arguments_give_the_design_text() {
    let mut host = TestHost::fixture();
    let this = host.create("Folder");
    let number = ScriptValue::Number(1.0);
    assert_eq!(
        host.text(this, "Required", &[]),
        "Folder:Required: argument #1 expected number, got nothing"
    );
    assert_eq!(
        host.text(this, "Required", std::slice::from_ref(&number)),
        "Folder:Required: argument #2 expected Enum.NormalId, got nothing"
    );
    assert_eq!(
        host.text(this, "Required", &[number.clone(), string("Top")]),
        "Folder:Required: argument #2 expected Enum.NormalId, got string"
    );
    assert_eq!(
        host.text(this, "Required", &[ScriptValue::Nil, ScriptValue::Nil]),
        "Folder:Required: argument #1 expected number, got nil"
    );
    // The count comes before any type, then the types left to right.
    assert_eq!(
        host.text(this, "Required", &[string("wrong")]),
        "Folder:Required: argument #2 expected Enum.NormalId, got nothing"
    );
    let wrong_enum = ScriptValue::EnumItem(EnumItemRef::new("Material", "Wood"));
    assert_eq!(
        host.text(this, "Required", &[number.clone(), wrong_enum]),
        "Folder:Required: argument #2 expected Enum.NormalId, got Enum.Material"
    );
    let item = ScriptValue::EnumItem(EnumItemRef::new("NormalId", "Top"));
    let too_many = [number, item, string("rest"), ScriptValue::Bool(true)];
    assert_eq!(
        host.text(this, "Required", &too_many),
        "Folder:Required: expected at most 3 arguments, got 4"
    );
    assert_eq!(
        host.text(this, "Whole", &[ScriptValue::Int(1), ScriptValue::Int(2)]),
        "Folder:Whole: expected at most 1 argument, got 2"
    );
    assert_eq!(
        host.err(this, "Required", &[]).kind,
        ApiErrorKind::MissingArgument {
            index: 1,
            expected: Ty::Number
        }
    );
}

#[test]
fn every_kind_of_wrong_value_is_named() {
    let mut host = TestHost::fixture();
    let this = host.create("Folder");
    let part = host.create("Part");
    for (value, name) in [
        (ScriptValue::Nil, "nil"),
        (ScriptValue::Bool(true), "boolean"),
        (string("text"), "string"),
        (ScriptValue::Instance(part), "Part"),
        (ScriptValue::Vector3(Vec3::new(0.0, 0.0, 0.0)), "Vector3"),
        (CFrame::IDENTITY.into(), "CFrame"),
        (ScriptValue::Color3(Color3::new(0.0, 0.0, 0.0)), "Color3"),
        (ScriptValue::UDim2(UDim2::new(0.0, 0.0, 0.0, 0.0)), "UDim2"),
        (ScriptValue::Array(Vec::new()), "array"),
        (ScriptValue::Map(Vec::new()), "map"),
        (ScriptValue::Function(FunctionRef(1)), "function"),
        (ScriptValue::Tuple(Vec::new()), "tuple"),
    ] {
        assert_eq!(
            host.text(this, "Whole", &[value]),
            format!("Folder:Whole: argument #1 expected integer, got {name}")
        );
    }
}

#[test]
fn instance_arguments_check_their_class_and_liveness() {
    let mut host = TestHost::fixture();
    let this = host.create("Folder");
    let folder = host.create("Folder");
    let part = host.create("Part");
    let gone = host.create("Folder");
    host.world.destroy(gone);
    host.ret(this, "OneFolder", &[folder.into()]);
    assert_eq!(
        host.text(this, "OneFolder", &[part.into()]),
        "Folder:OneFolder: argument #1 expected Folder, got Part"
    );
    assert_eq!(
        host.text(this, "OneFolder", &[gone.into()]),
        "Instance has been destroyed"
    );
    // A required `Optional` type: nil is fine, leaving it out is not.
    host.ret(this, "MaybePart", &[ScriptValue::Nil]);
    host.ret(this, "MaybePart", &[part.into()]);
    assert_eq!(
        host.text(this, "MaybePart", &[]),
        "Folder:MaybePart: argument #1 expected BasePart or nil, got nothing"
    );
    assert_eq!(
        host.text(this, "MaybePart", &[gone.into()]),
        "Instance has been destroyed"
    );
}

#[test]
fn variadic_arguments_are_counted_and_checked() {
    let mut host = TestHost::fixture();
    let this = host.create("Folder");
    let numbers = [
        string("first"),
        ScriptValue::Number(1.0),
        ScriptValue::Int(2),
    ];
    assert_eq!(host.ret(this, "Numbers", &numbers), ScriptValue::Int(3));
    assert_eq!(
        host.ret(this, "Numbers", &[string("first")]),
        ScriptValue::Int(1)
    );
    let wrong = [string("first"), ScriptValue::Number(1.0), string("no")];
    assert_eq!(
        host.text(this, "Numbers", &wrong),
        "Folder:Numbers: argument #3 expected number, got string"
    );
}

#[test]
fn a_manual_parameter_keeps_the_method_s_own_text() {
    let mut host = TestHost::fixture();
    let this = host.create("Folder");
    for args in [&[][..], &[string("no")][..]] {
        assert_eq!(host.text(this, "Manual", args), "Manual: expected a CFrame");
    }
    assert_eq!(
        host.ret(this, "Manual", &[CFrame::IDENTITY.into()]),
        CFrame::IDENTITY.into()
    );
    // A real one: PivotTo is declared Manual so its text stays (`tests/pivot.rs`).
    let pivot_to = classes::class_def("PVInstance")
        .unwrap()
        .methods
        .iter()
        .find(|method| method.name == "PivotTo")
        .unwrap();
    assert_eq!(
        classes::check_arguments(&host.world, "Part", pivot_to, &[]),
        Ok(())
    );
}

/// What a host does with [`WaitRequest::Child`]: parks the thread, and resumes it with the child
/// or, on timeout, with nil.
#[derive(Default)]
struct Parked {
    threads: VecDeque<(u32, WaitRequest)>,
    resumed: HashMap<u32, ScriptValue>,
}

impl Parked {
    fn step(&mut self, world: &World, timed_out: bool) {
        for (thread, request) in std::mem::take(&mut self.threads) {
            let WaitRequest::Child { parent, name, .. } = &request else {
                panic!("this test only parks on a child");
            };
            match world.find_first_child(*parent, name) {
                Some(child) => {
                    self.resumed.insert(thread, child.into());
                }
                None if timed_out => {
                    self.resumed.insert(thread, ScriptValue::Nil);
                }
                None => self.threads.push_back((thread, request)),
            }
        }
    }
}

#[test]
fn a_wait_outcome_round_trips_through_the_host() {
    let mut host = TestHost::fixture();
    let this = host.create("Folder");
    let args = [string("Child"), ScriptValue::Number(5.0)];
    let outcome = host.call(this, "Instance", "WaitForIt", &args).unwrap();
    let request = WaitRequest::Child {
        parent: this,
        name: "Child".into(),
        timeout: Some(5.0),
    };
    assert_eq!(outcome, CallOutcome::Wait(request.clone()));

    let mut parked = Parked::default();
    parked.threads.push_back((1, request.clone()));
    parked.threads.push_back((2, request));
    parked.step(&host.world, false);
    assert!(parked.resumed.is_empty(), "no such child yet");

    let child = host.world.create("Folder", Some(this)).unwrap();
    host.world
        .set(child, "Name", Value::String("Child".into()))
        .unwrap();
    parked.step(&host.world, false);
    assert_eq!(parked.resumed[&1], ScriptValue::Instance(child));
    assert_eq!(parked.resumed[&2], ScriptValue::Instance(child));

    // No such child: the timeout resumes with nil.
    let other = host.call(this, "Instance", "WaitForIt", &[string("Nope")]);
    let Ok(CallOutcome::Wait(request)) = other else {
        panic!("{other:?}");
    };
    parked.threads.push_back((3, request));
    parked.step(&host.world, true);
    assert_eq!(parked.resumed[&3], ScriptValue::Nil);
}

#[test]
fn every_wait_request_compares_by_value() {
    let id = InstanceId::from_bits(1);
    let key = SignalKey {
        instance: id,
        signal: MemberId(2),
        arg: 0,
    };
    let requests = [
        WaitRequest::Child {
            parent: id,
            name: "A".into(),
            timeout: None,
        },
        WaitRequest::Signal { key },
        WaitRequest::Reply { request: 9 },
        WaitRequest::Seconds(0.5),
    ];
    for (i, request) in requests.iter().enumerate() {
        for (j, other) in requests.iter().enumerate() {
            assert_eq!(i == j, request == other);
        }
    }
}

#[test]
fn only_a_method_declared_as_yielding_may_wait() {
    let mut host = TestHost::fixture();
    let this = host.create("Folder");
    assert_eq!(
        host.text(this, "WaitsUndeclared", &[]),
        "WaitsUndeclared waited but is not declared as yielding"
    );
}

#[test]
fn a_method_declared_on_a_superclass_is_callable_on_a_subclass() {
    // Native, on the fixture: errors and the call name the instance's class, as `Part:Resize` does.
    let mut host = TestHost::fixture();
    let folder = host.create("Folder");
    let part = host.create("Part");
    assert_eq!(host.ret(part, "WhereAmI", &[]), "Part".into());
    assert_eq!(host.ret(folder, "WhereAmI", &[]), "Folder".into());
    // Address it through the subclass too: same member by the prefix rule.
    let base = host.resolve_class("Instance").unwrap();
    let sub = host.resolve_class("Folder").unwrap();
    assert_eq!(
        host.resolve_member(base, "WhereAmI"),
        host.resolve_member(sub, "WhereAmI")
    );
    assert_eq!(
        host.call(folder, "Folder", "WhereAmI", &[]),
        Ok("Folder".into())
    );

    // Host, on the real registry: PVInstance declares GetPivot, Part and Model inherit it.
    let mut host = TestHost::real();
    let part = host.create("Part");
    let position = Vec3::new(1.0, 2.0, 3.0);
    host.world
        .set(part, "Position", Value::Vector3(position))
        .unwrap();
    let CallOutcome::Return(pivot) = host.call(part, "PVInstance", "GetPivot", &[]).unwrap() else {
        panic!("GetPivot does not wait");
    };
    assert_eq!(pivot.as_cframe().unwrap().position, position);
    let model = host.create("Model");
    assert!(host.call(model, "PVInstance", "GetPivot", &[]).is_ok());
}

#[test]
fn the_nearest_declaration_runs_and_the_superclass_keeps_its_own() {
    let mut host = TestHost::fixture();
    let folder = host.create("Folder");
    let part = host.create("Part");
    // The wrapper knows the base class; the instance's own table decides.
    assert_eq!(host.ret(folder, "Version", &[]), "sub".into());
    assert_eq!(host.ret(part, "Version", &[]), "base".into());
}

#[test]
fn the_host_refuses_what_the_instance_does_not_have() {
    let mut host = TestHost::fixture();
    let folder = host.create("Folder");
    let part = host.create("Part");
    // Hidden from this class.
    assert_eq!(
        host.text(folder, "HiddenBelow", &[]),
        "HiddenBelow is not a valid member of Folder"
    );
    host.ret(part, "HiddenBelow", &[]);
    // An instance that is not the class the wrapper names.
    let sub = host.resolve_class("Folder").unwrap();
    let version = host.resolve_member(sub, "Version").unwrap();
    assert_eq!(
        host.call_method(part, sub, version, &[])
            .unwrap_err()
            .to_string(),
        "Version is not a valid member of Part"
    );
    // An id no class hands out.
    assert_eq!(
        host.call_method(part, sub, MemberId(9999), &[])
            .unwrap_err()
            .to_string(),
        "member is not a valid member of Part"
    );
    // A destroyed instance.
    host.world.destroy(folder);
    assert_eq!(
        host.text(folder, "WhereAmI", &[]),
        "Instance has been destroyed"
    );
    assert!(matches!(
        host.err(folder, "WhereAmI", &[]).kind,
        ApiErrorKind::Destroyed { .. }
    ));
}

#[test]
fn a_property_is_not_callable_through_the_method_path() {
    let mut host = TestHost::real();
    let part = host.create("Part");
    assert_eq!(
        host.call(part, "Instance", "Name", &[])
            .unwrap_err()
            .to_string(),
        "Name is not a method"
    );
}

#[test]
fn a_host_method_gets_the_host_state_and_checked_arguments() {
    let mut host = TestHost::fixture();
    let this = host.create("Folder");
    assert_eq!(
        host.ret(this, "HostMethod", &[ScriptValue::Number(4.0)]),
        4.0.into()
    );
    assert_eq!(host.state.calls, 1);
    assert_eq!(
        host.text(this, "HostMethod", &[string("no")]),
        "Folder:HostMethod: argument #1 expected number, got string"
    );
    assert_eq!(host.state.calls, 1, "the body did not run");
    // Declared, no body: an error at the call, and the completeness test catches it earlier.
    assert_eq!(
        host.text(this, "Unimplemented", &[]),
        "Unimplemented has no host implementation"
    );
    assert_eq!(
        host.text(this, "InLuau", &[]),
        "InLuau is implemented in Luau; the binding calls it"
    );
}

#[test]
fn a_declared_host_method_without_an_implementation_fails_completeness() {
    let mut methods = HostMethods::<State>::new(&FIXTURE);
    let missing = |method| HostMethodError::Missing {
        class: "Instance",
        method,
    };
    assert_eq!(methods.check_complete(&[]), Err(missing("HostMethod")));
    assert_eq!(
        methods.check_complete(&[("Instance", "HostMethod")]),
        Err(missing("Unimplemented"))
    );
    methods
        .implement("Instance", "HostMethod", host_body)
        .unwrap();
    assert_eq!(methods.check_complete(&[]), Err(missing("Unimplemented")));
    methods
        .implement("Instance", "Unimplemented", host_body)
        .unwrap();
    assert_eq!(methods.check_complete(&[]), Ok(()));
    assert_eq!(
        missing("HostMethod").to_string(),
        "Instance.HostMethod is declared Host but has no implementation"
    );
}

#[test]
fn an_implementation_must_name_a_host_method_its_class_declares() {
    let mut methods = HostMethods::<State>::new(&FIXTURE);
    for (class, method) in [
        ("Instance", "NoSuchMethod"),
        ("NoSuchClass", "HostMethod"),
        // Declared Native.
        ("Instance", "Version"),
        // Inherited, not declared there.
        ("Folder", "HostMethod"),
        // Declared Luau.
        ("Instance", "InLuau"),
    ] {
        assert_eq!(
            methods.implement(class, method, host_body),
            Err(HostMethodError::NotDeclared { class, method })
        );
    }
    methods
        .implement("Instance", "HostMethod", host_body)
        .unwrap();
    assert_eq!(
        methods.implement("Instance", "HostMethod", host_body),
        Err(HostMethodError::ImplementedTwice {
            class: "Instance",
            method: "HostMethod",
        })
    );
    assert_eq!(
        methods.check_complete(&[("Instance", "HostMethod"), ("Instance", "Unimplemented")]),
        Err(HostMethodError::NeedlessEntry {
            class: "Instance",
            method: "HostMethod",
        })
    );
}

#[test]
fn an_override_needs_its_own_host_body() {
    static BASE: ClassDesc = ClassDesc::new("Instance", None).methods(&[MethodDesc::host("M")]);
    static SUB: ClassDesc =
        ClassDesc::new("Folder", Some("Instance")).methods(&[MethodDesc::host("M").overrides()]);
    let registry = classes::resolve(&[&BASE, &SUB]).unwrap();
    let mut methods = HostMethods::<State>::new(&registry);
    methods.implement("Instance", "M", host_body).unwrap();
    assert_eq!(
        methods.check_complete(&[]),
        Err(HostMethodError::Missing {
            class: "Folder",
            method: "M",
        })
    );
    methods.implement("Folder", "M", host_body).unwrap();
    assert_eq!(methods.check_complete(&[]), Ok(()));
}

#[test]
fn a_world_error_raised_by_a_method_keeps_the_world_s_text() {
    let mut host = TestHost::fixture();
    let this = host.create("Folder");
    let error = host.err(this, "WorldRefuses", &[]);
    assert_eq!(error.class, "Folder");
    assert_eq!(error.member, "WorldRefuses");
    assert_eq!(
        error.to_string(),
        "class `Folder` has no property `NoSuchProperty`"
    );
    assert!(matches!(error.kind, ApiErrorKind::Core(_)));
}

#[test]
fn the_formatter_keeps_today_s_texts() {
    let text = |class, member: &'static str, kind| ApiError::new(class, member, kind).to_string();
    assert_eq!(
        text("Model", "Missing", ApiErrorKind::NotAMember),
        "Missing is not a valid member of Model"
    );
    assert_eq!(
        text("Part", "ClassName", ApiErrorKind::ReadOnly),
        "ClassName is read-only"
    );
    assert_eq!(
        text(
            "RemoteFunction",
            "OnServerInvoke",
            ApiErrorKind::CallbackRead
        ),
        "OnServerInvoke is a callback; it can only be set"
    );
    let wrong = |required, member_kind| ApiErrorKind::WrongContext {
        required,
        member_kind,
    };
    assert_eq!(
        text(
            "RemoteEvent",
            "FireServer",
            wrong(Side::Client, MemberCategory::Method)
        ),
        "FireServer can only be called from the client"
    );
    assert_eq!(
        text(
            "RemoteEvent",
            "FireClient",
            wrong(Side::Server, MemberCategory::Method)
        ),
        "FireClient can only be called from the server"
    );
    assert_eq!(
        text(
            "RemoteEvent",
            "OnServerEvent",
            wrong(Side::Server, MemberCategory::Signal)
        ),
        "OnServerEvent can only be used from the server"
    );
    assert_eq!(
        text(
            "",
            "",
            ApiErrorKind::NotCreatable {
                class: "Workspace".into()
            }
        ),
        "Unable to create an Instance of type \"Workspace\""
    );
    assert_eq!(
        text(
            "Part",
            "Name",
            ApiErrorKind::Destroyed {
                name: "Crate".into()
            }
        ),
        "Crate has been destroyed"
    );
    assert_eq!(
        ApiError::message("PivotTo: expected a CFrame").to_string(),
        "PivotTo: expected a CFrame"
    );
}

#[test]
fn types_print_in_one_language_neutral_form() {
    for (ty, text) in [
        (Ty::Nil, "nil"),
        (Ty::Bool, "boolean"),
        (Ty::Int, "integer"),
        (Ty::Number, "number"),
        (Ty::String, "string"),
        (Ty::Instance(None), "Instance"),
        (Ty::Instance(Some("BasePart")), "BasePart"),
        (Ty::Vector3, "Vector3"),
        (Ty::CFrame, "CFrame"),
        (Ty::Color3, "Color3"),
        (Ty::UDim2, "UDim2"),
        (Ty::Enum("NormalId"), "Enum.NormalId"),
        (Ty::Array(&Ty::Number), "array of number"),
        (Ty::Map(&Ty::Bool), "map of boolean"),
        (Ty::Function(&ANY_FUNCTION), "function"),
        (Ty::Optional(&Ty::String), "string or nil"),
        (Ty::Tuple(&[Ty::Bool, Ty::Nil]), "tuple"),
        (Ty::Any, "any"),
    ] {
        assert_eq!(ty.to_string(), text);
    }
}

#[test]
fn nested_types_conform_element_by_element() {
    let mut world = World::new_with_services();
    let parent = world.root("Workspace");
    let part = world.create("Part", parent).unwrap();
    let conforms = |ty: Ty, value: ScriptValue| classes::conforms(&world, &ty, &value);
    assert!(conforms(Ty::Any, ScriptValue::Nil));
    assert!(conforms(Ty::Nil, ScriptValue::Nil));
    assert!(!conforms(Ty::Nil, ScriptValue::Bool(false)));
    assert!(conforms(Ty::Instance(Some("BasePart")), part.into()));
    assert!(!conforms(
        Ty::Array(&Ty::Number),
        ScriptValue::Array(vec!["no".into()])
    ));
    // A map's keys are strings.
    assert!(!conforms(
        Ty::Map(&Ty::Number),
        ScriptValue::Map(vec![(ScriptValue::Int(1), ScriptValue::Int(1))])
    ));
    assert!(conforms(
        Ty::Tuple(&[Ty::Bool, Ty::Number]),
        ScriptValue::Tuple(vec![true.into(), ScriptValue::Int(1)])
    ));
    assert!(!conforms(
        Ty::Tuple(&[Ty::Bool, Ty::Number]),
        ScriptValue::Tuple(vec![true.into()])
    ));
    assert!(!conforms(Ty::Tuple(&[Ty::Bool]), true.into()));
}

#[test]
fn every_value_converts_to_a_script_value_and_back() {
    let id = InstanceId::from_bits(5);
    for (value, ty) in [
        (Value::Nil, metallic_core::ValueType::Nil),
        (Value::Bool(true), metallic_core::ValueType::Bool),
        (Value::Number(1.5), metallic_core::ValueType::Number),
        (
            Value::String("text".into()),
            metallic_core::ValueType::String,
        ),
        (
            Value::Vector3(Vec3::new(1.0, 2.0, 3.0)),
            metallic_core::ValueType::Vector3,
        ),
        (
            Value::Color3(Color3::new(1.0, 2.0, 3.0)),
            metallic_core::ValueType::Color3,
        ),
        (
            Value::UDim2(UDim2::new(1.0, 2.0, 3.0, 4.0)),
            metallic_core::ValueType::UDim2,
        ),
        (Value::Ref(None), metallic_core::ValueType::Ref),
        (Value::Ref(Some(id)), metallic_core::ValueType::Ref),
        (
            Value::CFrame(Box::new(CFrame::IDENTITY)),
            metallic_core::ValueType::CFrame,
        ),
    ] {
        let script = ScriptValue::from(&value);
        assert_eq!(script.to_value(ty), Some(value));
    }
    // The reverse is checked against the property's type.
    assert_eq!(
        ScriptValue::Int(2).to_value(metallic_core::ValueType::Number),
        Some(Value::Number(2.0))
    );
    assert_eq!(
        ScriptValue::Number(2.0).to_value(metallic_core::ValueType::String),
        None
    );
    assert_eq!(
        ScriptValue::Nil.to_value(metallic_core::ValueType::Number),
        None
    );
    assert_eq!(
        ScriptValue::Array(Vec::new()).to_value(metallic_core::ValueType::Nil),
        None
    );
}

#[test]
fn the_rest_of_the_host_interface_works_by_id() {
    let mut host = TestHost::real();
    let part = host.create("Part");
    let instance = host.resolve_class("Instance").unwrap();
    let part_class = host.resolve_class("Part").unwrap();
    assert_eq!(host.class_of(part), Ok(part_class));
    assert_eq!(host.is_a(part, instance), Ok(true));
    let folder_class = host.resolve_class("Folder").unwrap();
    assert_eq!(host.is_a(part, folder_class), Ok(false));
    assert_eq!(host.resolve_class("NoSuchClass"), None);
    assert_eq!(host.resolve_member(instance, "NoSuchMember"), None);

    // A property read and written through the class that declares it.
    let name = host.resolve_member(instance, "Name").unwrap();
    assert_eq!(host.get_property(part, instance, name), Ok("Part".into()));
    host.set_property(part, instance, name, "Crate".into())
        .unwrap();
    assert_eq!(host.get_property(part, instance, name), Ok("Crate".into()));
    let wrong = host
        .set_property(part, instance, name, true.into())
        .unwrap_err();
    assert_eq!(
        wrong.to_string(),
        "property `Name` expects String, got boolean"
    );

    let made = host.new_instance(folder_class, Some(part)).unwrap();
    assert_eq!(host.world.parent(made), Some(part));
    let workspace = host.resolve_class("Workspace").unwrap();
    assert_eq!(
        host.new_instance(workspace, None).unwrap_err().to_string(),
        "Unable to create an Instance of type \"Workspace\""
    );

    let gone = host.create("Folder");
    host.world.destroy(gone);
    assert_eq!(
        host.get_property(gone, instance, name)
            .unwrap_err()
            .to_string(),
        "Instance has been destroyed"
    );
    assert!(host.class_of(gone).is_err());
}

#[test]
fn a_method_reads_the_context_it_was_called_with() {
    fn side(call: &mut MethodCall) -> Result<CallOutcome, ApiError> {
        Ok((call.context.side == Side::Client).into())
    }
    static CLASS: ClassDesc =
        ClassDesc::new("Instance", None).methods(&[MethodDesc::native("Side", side)]);
    static FOLDER: ClassDesc = ClassDesc::new("Folder", Some("Instance"));
    let registry: &'static Registry =
        Box::leak(Box::new(classes::resolve(&[&CLASS, &FOLDER]).unwrap()));
    let mut host = TestHost::with(registry, HostMethods::new(registry));
    host.context.side = Side::Client;
    let this = host.create("Folder");
    assert_eq!(host.ret(this, "Side", &[]), true.into());
}

/// The crates `root` reaches in `Cargo.lock`, itself and dev-dependencies included.
fn reachable<'a>(lock: &'a str, root: &'a str) -> HashSet<&'a str> {
    let mut dependencies: HashMap<&str, Vec<&str>> = HashMap::new();
    for package in lock.split("[[package]]").skip(1) {
        let mut lines = package.lines();
        let Some(name) = lines
            .by_ref()
            .find_map(|line| line.trim().strip_prefix("name = \""))
            .and_then(|rest| rest.strip_suffix('"'))
        else {
            continue;
        };
        let listed = lines
            .skip_while(|line| !line.starts_with("dependencies = ["))
            .skip(1)
            .take_while(|line| !line.starts_with(']'))
            .filter_map(|line| line.trim().trim_matches(['"', ',']).split(' ').next());
        dependencies.entry(name).or_default().extend(listed);
    }
    let mut seen = HashSet::new();
    let mut queue = vec![root];
    while let Some(name) = queue.pop() {
        if seen.insert(name) {
            queue.extend(dependencies.get(name).into_iter().flatten().copied());
        }
    }
    seen
}

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn core_stays_free_of_any_scripting_language() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lock = std::fs::read_to_string(manifest.join("../../Cargo.lock")).unwrap();
    let reached = reachable(&lock, "metallic_core");
    assert!(reached.contains("sha2"), "the walk found nothing");
    // The same walk does see it where it belongs.
    assert!(reachable(&lock, "metallic_script").contains("mlua"));
    assert!(!reached.contains("metallic_script"));
    for forbidden in ["mlua", "mlua-sys", "wasmtime"] {
        assert!(
            !reached.contains(forbidden),
            "metallic_core reaches {forbidden}"
        );
    }
    let mut files = Vec::new();
    rust_files(&manifest.join("src"), &mut files);
    assert!(files.len() > 50);
    for file in files {
        let source = std::fs::read_to_string(&file).unwrap();
        for word in ["mlua::", "wasmtime::"] {
            assert!(!source.contains(word), "{} uses {word}", file.display());
        }
    }
}
