use wasmi::*;

// In this simple example we are going to compile the below Wasm source,
// instantiate a Wasm module from it and call its exported "hello" function.
fn main() {
    let wasm = r#"
        (module
            (import "host" "hello" (func $host_hello (param i32)))
            (func (export "hello")
                (call $host_hello (i32.const 3))
            )
        )
    "#;
    // First step is to create the Wasm execution engine with some config.
    //
    // In this example we are using the default configuration.
    let engine = Engine::default();
    // Now we can compile the above Wasm module with the given Wasm source.
    let module = Module::new(
        &engine,
        include_bytes!("../../wasm_code/target/wasm32-unknown-unknown/release/wasm_code.wasm"),
    )
    .unwrap();

    // Wasm objects operate within the context of a Wasm `Store`.
    //
    // Each `Store` has a type parameter to store host specific data.
    // In this example the host state is a simple `u32` type with value `42`.
    type HostState = u32;
    let mut store = Store::new(&engine, 42);

    // A linker can be used to instantiate Wasm modules.
    // The job of a linker is to satisfy the Wasm module's imports.
    let mut linker = <Linker<HostState>>::new(&engine);
    // We are required to define all imports before instantiating a Wasm module.
    linker
        .func_wrap(
            "host",
            "hello",
            |caller: Caller<'_, HostState>, param: i32| {
                println!(
                    "Got {param} from WebAssembly and my host state is: {}",
                    caller.data()
                );
            },
        )
        .unwrap();
    let instance = linker.instantiate_and_start(&mut store, &module).unwrap();
    // Now we can finally query the exported "hello" function and call it.
    let magic_number = instance
        .get_typed_func::<(), i32>(&store, "hello")
        .unwrap()
        .call(&mut store, ())
        .unwrap();
    println!("the magic number is {}", magic_number);
}
