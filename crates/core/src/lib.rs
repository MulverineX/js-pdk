use quickjs_runtime::builder::QuickJsRuntimeBuilder;
use quickjs_runtime::facades::QuickJsRuntimeFacade;
use quickjs_runtime::jsutils::{JsError, Script};
use quickjs_runtime::quickjs_utils::{functions, objects};
use quickjs_runtime::quickjsrealmadapter::QuickJsRealmAdapter;
use quickjs_runtime::quickjsruntimeadapter::QuickJsRuntimeAdapter;
use quickjs_runtime::quickjsvalueadapter::QuickJsValueAdapter;
use quickjs_runtime::values::JsValueFacade;
use std::io::{self, Read};

mod globals;

static RUNTIME: std::sync::OnceLock<QuickJsRuntimeFacade> = std::sync::OnceLock::new();
static CALL_ARGS: std::sync::Mutex<Vec<Vec<JsValueFacade>>> = std::sync::Mutex::new(vec![]);

fn js_error_to_string(err: JsError) -> String {
    format!("{}\n{}", err.get_message(), err.get_stack())
}

fn invoke(idx: i32) -> Result<JsValueFacade, JsError> {
    let call_args = CALL_ARGS.lock().unwrap().pop().unwrap();
    let rt = RUNTIME.get().expect("Runtime not initialized");

    rt.loop_realm_sync(None, move |_rt: &QuickJsRuntimeAdapter, realm: &QuickJsRealmAdapter| {
        // Convert args from JsValueFacade to QuickJsValueAdapter
        let args: Vec<QuickJsValueAdapter> = call_args
            .into_iter()
            .filter_map(|facade| realm.from_js_value_facade(facade).ok())
            .collect();

        // Get module.exports
        let global = realm.get_global()?;
        let module = objects::get_property_q(realm, &global, "module")?;
        let exports = objects::get_property_q(realm, &module, "exports")?;

        // Get export names and sort them
        let export_names = get_export_names(realm, &exports)?;
        let export_name = export_names.get(idx as usize).ok_or_else(|| {
            JsError::new_string(format!("Export index {} out of range", idx))
        })?;

        // Get the function
        let func = objects::get_property_q(realm, &exports, export_name)?;

        // Call the function
        let result = functions::call_function_q(realm, &func, &args, None)?;

        // Check for exception
        if result.is_exception() {
            unsafe {
                if let Some(ex_err) = QuickJsRealmAdapter::get_exception(realm.context) {
                    return Err(ex_err);
                }
            }
            return Err(JsError::new_string("Function call failed with exception".to_string()));
        }

        realm.to_js_value_facade(&result)
    })
}

fn get_export_names(realm: &QuickJsRealmAdapter, exports: &QuickJsValueAdapter) -> Result<Vec<String>, JsError> {
    objects::get_property_names_q(realm, exports)
}

#[export_name = "wizer.initialize"]
extern "C" fn init() {
    let rt = QuickJsRuntimeBuilder::new().build();
    let _ = RUNTIME.set(rt);

    let rt = RUNTIME.get().expect("Runtime not initialized");

    // Read JS code from stdin
    let mut code = String::new();
    io::stdin().read_to_string(&mut code).unwrap();

    // Inject globals and eval code
    rt.loop_realm_sync(None, move |_rt: &QuickJsRuntimeAdapter, realm: &QuickJsRealmAdapter| {
        globals::inject_globals(realm).map_err(|e| JsError::new_string(format!("Failed to inject globals: {}", e)))?;

        // Eval the user code
        realm.eval(Script::new("input.js", &code)).map_err(|e| {
            JsError::new_string(format!("Eval failed: {}", e.get_message()))
        })?;

        Ok::<(), JsError>(())
    }).expect("Initialization failed");
}

#[no_mangle]
pub extern "C" fn __arg_start() {
    CALL_ARGS.lock().unwrap().push(vec![]);
}

#[no_mangle]
pub extern "C" fn __arg_i32(arg: i32) {
    CALL_ARGS
        .lock()
        .unwrap()
        .last_mut()
        .unwrap()
        .push(JsValueFacade::I32 { val: arg });
}

#[no_mangle]
pub extern "C" fn __arg_i64(arg: i64) {
    CALL_ARGS
        .lock()
        .unwrap()
        .last_mut()
        .unwrap()
        .push(JsValueFacade::F64 { val: arg as f64 });
}

#[no_mangle]
pub extern "C" fn __arg_f32(arg: f32) {
    CALL_ARGS
        .lock()
        .unwrap()
        .last_mut()
        .unwrap()
        .push(JsValueFacade::F64 { val: arg as f64 });
}

#[no_mangle]
pub extern "C" fn __arg_f64(arg: f64) {
    CALL_ARGS
        .lock()
        .unwrap()
        .last_mut()
        .unwrap()
        .push(JsValueFacade::F64 { val: arg });
}

#[no_mangle]
pub extern "C" fn __invoke_i32(idx: i32) -> i32 {
    match invoke(idx) {
        Ok(v) => v.get_i32(),
        Err(e) => {
            let mem = extism_pdk::Memory::from_bytes(&js_error_to_string(e)).unwrap();
            unsafe { extism_pdk::extism::error_set(mem.offset()) }
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn __invoke_i64(idx: i32) -> i64 {
    match invoke(idx) {
        Ok(v) => v.get_i64(),
        Err(e) => {
            let mem = extism_pdk::Memory::from_bytes(&js_error_to_string(e)).unwrap();
            unsafe { extism_pdk::extism::error_set(mem.offset()) }
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn __invoke_f64(idx: i32) -> f64 {
    match invoke(idx) {
        Ok(v) => v.get_f64(),
        Err(e) => {
            let mem = extism_pdk::Memory::from_bytes(&js_error_to_string(e)).unwrap();
            unsafe { extism_pdk::extism::error_set(mem.offset()) }
            -1.0
        }
    }
}

#[no_mangle]
pub extern "C" fn __invoke_f32(idx: i32) -> f32 {
    match invoke(idx) {
        Ok(v) => v.get_f64() as f32,
        Err(e) => {
            let mem = extism_pdk::Memory::from_bytes(&js_error_to_string(e)).unwrap();
            unsafe { extism_pdk::extism::error_set(mem.offset()) }
            -1.0
        }
    }
}

#[no_mangle]
pub extern "C" fn __invoke(idx: i32) {
    if let Err(e) = invoke(idx) {
        let mem = extism_pdk::Memory::from_bytes(&js_error_to_string(e)).unwrap();
        unsafe { extism_pdk::extism::error_set(mem.offset()) }
    }
}