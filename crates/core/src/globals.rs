use std::str::from_utf8;

use chrono::{SecondsFormat, Utc};
use extism_pdk::extism::load_input;
use extism_pdk::*;
use quickjs_runtime::jsutils::{JsError, Script};
use quickjs_runtime::quickjsrealmadapter::QuickJsRealmAdapter;
use quickjs_runtime::quickjs_utils::new_undefined_ref;
use quickjs_runtime::quickjs_utils::new_null_ref;
use quickjs_runtime::quickjs_utils::arrays::{get_length_q, self as arrays_module};
use quickjs_runtime::quickjs_utils::typedarrays::{get_array_buffer_buffer_copy_q, is_array_buffer_q, new_uint8_array_copy_q};
use quickjs_runtime::quickjs_utils::{functions, objects, primitives};
use quickjs_runtime::quickjsvalueadapter::QuickJsValueAdapter;
use sha2::Digest;

static PRELUDE: &[u8] = include_bytes!("prelude/dist/index.js");

pub fn inject_globals(realm: &QuickJsRealmAdapter) -> Result<(), JsError> {
    // Build global objects
    let module = build_module_object(realm)?;
    let console_write = build_console_writer(realm)?;
    let var_obj = build_var_object(realm)?;
    let http_obj = build_http_object(realm)?;
    let config_obj = build_config_object(realm)?;
    let decoder = build_decoder(realm)?;
    let encoder = build_encoder(realm)?;
    let clock = build_clock(realm)?;
    let clock_ms = build_clock_ms(realm)?;
    let random_bytes = build_random_bytes(realm)?;
    let sha_digest = build_sha_digest(realm)?;
    let mem = build_memory(realm)?;
    let host = build_host_object(realm)?;

    // Set globals
    let global = realm.get_global()?;
    realm.set_object_property(&global, "__consoleWrite", &console_write)?;
    realm.set_object_property(&global, "module", &module)?;
    realm.set_object_property(&global, "Host", &host)?;
    realm.set_object_property(&global, "Var", &var_obj)?;
    realm.set_object_property(&global, "Http", &http_obj)?;
    realm.set_object_property(&global, "Config", &config_obj)?;
    realm.set_object_property(&global, "Memory", &mem)?;
    realm.set_object_property(&global, "__decodeUtf8BufferToString", &decoder)?;
    realm.set_object_property(&global, "__encodeStringToUtf8Buffer", &encoder)?;
    realm.set_object_property(&global, "__getTime", &clock)?;
    realm.set_object_property(&global, "__getTimeMs", &clock_ms)?;
    realm.set_object_property(&global, "__getRandomBytes", &random_bytes)?;
    realm.set_object_property(&global, "__shaDigest", &sha_digest)?;

    add_host_functions(realm)?;

    realm.eval(Script::new("init_module.js", "globalThis.module = {}; globalThis.module.exports = {}"))?;
    realm.eval(Script::new("init_global.js", "var global = globalThis"))?;
    realm.eval(Script::new("prelude.js", from_utf8(PRELUDE).map_err(|e| JsError::new_string(format!("Utf8 error: {}", e)))?))?;

    Ok(())
}

#[link(wasm_import_module = "shim")]
extern "C" {
    fn __invokeHostFunc(
        func_idx: u32,
        arg0: u64,
        arg1: u64,
        arg2: u64,
        arg3: u64,
        arg4: u64,
    ) -> u64;
    fn __get_function_return_type(func_idx: u32) -> u32;
    fn __get_function_arg_type(func_idx: u32, arg_idx: u32) -> u32;
}

fn build_console_writer(realm: &QuickJsRealmAdapter) -> Result<QuickJsValueAdapter, JsError> {
    realm.create_function("__consoleWrite", move |realm, _this, args| {
        if args.len() != 2 {
            return Err(JsError::new_string("Expected level and message arguments".to_string()));
        }

        let level = primitives::to_string_q(realm, &args[0])?;
        let message = primitives::to_string_q(realm, &args[1])?;

        match level.as_str() {
            "info" | "log" => info!("{}", message),
            "warn" => warn!("{}", message),
            "error" => error!("{}", message),
            "debug" => debug!("{}", message),
            "trace" => trace!("{}", message),
            _ => warn!("{}", message),
        }

        Ok(primitives::from_i32(0))
    }, 2)
}

fn build_module_object(realm: &QuickJsRealmAdapter) -> Result<QuickJsValueAdapter, JsError> {
    let exports = realm.create_object()?;
    let module = realm.create_object()?;
    realm.set_object_property(&module, "exports", &exports)?;
    Ok(module)
}

fn build_host_object(realm: &QuickJsRealmAdapter) -> Result<QuickJsValueAdapter, JsError> {
    let host_input_bytes = realm.create_function("inputBytes", move |_realm, _this, _args| {
        let input = unsafe { load_input() };
        new_uint8_array_copy_q(_realm, &input)
    }, 0)?;

    let host_input_string = realm.create_function("inputString", move |_realm, _this, _args| {
        let input = unsafe { load_input() };
        let input_string = String::from_utf8(input)
            .map_err(|e| JsError::new_string(format!("Utf8 error: {}", e)))?;
        primitives::from_string_q(_realm, &input_string)
    }, 0)?;

    let host_output_bytes = realm.create_function("outputBytes", move |realm, _this, args| {
        let output = args.first()
            .ok_or_else(|| JsError::new_string("Expected output argument".to_string()))?;
        let bytes = get_array_buffer_buffer_copy_q(realm, output)?;
        extism_pdk::output(&bytes).map_err(|e| JsError::new_string(format!("Output error: {}", e)))?;
        Ok(primitives::from_bool(true))
    }, 1)?;

    let host_output_string = realm.create_function("outputString", move |realm, _this, args| {
        let output = args.first()
            .ok_or_else(|| JsError::new_string("Expected output argument".to_string()))?;
        let output_string = primitives::to_string_q(realm, output)?;
        extism_pdk::output(output_string).map_err(|e| JsError::new_string(format!("Output error: {}", e)))?;
        Ok(primitives::from_bool(true))
    }, 1)?;

    let to_base64 = realm.create_function("arrayBufferToBase64", move |realm, _this, args| {
        let data = args.first()
            .ok_or_else(|| JsError::new_string("Expected data argument".to_string()))?;
        let bytes = get_array_buffer_buffer_copy_q(realm, data)
            .map_err(|e| JsError::new_string(format!("Expected ArrayBuffer: {}", e)))?;
        use base64::prelude::*;
        let as_string = BASE64_STANDARD.encode(&bytes);
        primitives::from_string_q(realm, &as_string)
    }, 1)?;

    let from_base64 = realm.create_function("base64ToArrayBuffer", move |realm, _this, args| {
        let data = args.first()
            .ok_or_else(|| JsError::new_string("Expected string argument".to_string()))?;
        let string = primitives::to_string_q(realm, data)?;
        use base64::prelude::*;
        let bytes = BASE64_STANDARD.decode(string)
            .map_err(|e| JsError::new_string(format!("Base64 decode error: {}", e)))?;
        new_uint8_array_copy_q(realm, &bytes)
    }, 1)?;

    let host_object = realm.create_object()?;
    realm.set_object_property(&host_object, "inputBytes", &host_input_bytes)?;
    realm.set_object_property(&host_object, "inputString", &host_input_string)?;
    realm.set_object_property(&host_object, "outputBytes", &host_output_bytes)?;
    realm.set_object_property(&host_object, "outputString", &host_output_string)?;
    realm.set_object_property(&host_object, "arrayBufferToBase64", &to_base64)?;
    realm.set_object_property(&host_object, "base64ToArrayBuffer", &from_base64)?;
    Ok(host_object)
}

fn add_host_functions(realm: &QuickJsRealmAdapter) -> Result<(), JsError> {
    let global = realm.get_global()?;
    let host_object = realm.get_object_property(&global, "Host")?;

    let host_invoke = realm.create_function("invokeFunc", move |realm, _this, args| -> Result<QuickJsValueAdapter, JsError> {
        let func_id = args.first()
            .ok_or_else(|| JsError::new_string("Expected function id argument".to_string()))?;
        let func_id = primitives::to_i32(func_id)? as u32;
        let mut params = [0u64; 5];

        for i in 1..args.len() {
            let arg = &args[i];
            params[i - 1] = convert_to_u64_bits(realm, arg, func_id, (i - 1) as u32)?;
        }

        let result = unsafe {
            __invokeHostFunc(
                func_id, params[0], params[1], params[2], params[3], params[4],
            )
        };

        let return_type = unsafe { __get_function_return_type(func_id) };
        match return_type {
            TYPE_VOID => Ok(new_undefined_ref()),
            TYPE_I32 => Ok(primitives::from_i32((result & 0xFFFFFFFF) as i32)),
            TYPE_I64 => Ok(primitives::from_f64(result as f64)),
            TYPE_F32 => Ok(primitives::from_f64(f32::from_bits(result as u32) as f64)),
            TYPE_F64 => Ok(primitives::from_f64(f64::from_bits(result))),
            _ => Err(JsError::new_string(format!("Unsupported return type: {:?}", return_type))),
        }
    }, 6)?;

    realm.set_object_property(&host_object, "invokeFunc", &host_invoke)?;
    Ok(())
}

const TYPE_VOID: u32 = 0;
const TYPE_I32: u32 = 1;
const TYPE_I64: u32 = 2;
const TYPE_F32: u32 = 3;
const TYPE_F64: u32 = 4;

fn convert_to_u64_bits(realm: &QuickJsRealmAdapter, value: &QuickJsValueAdapter, func_id: u32, arg_idx: u32) -> Result<u64, JsError> {
    match unsafe { __get_function_arg_type(func_id, arg_idx) } {
        TYPE_I32 => {
            let n = primitives::to_f64(value)?;
            Ok(n as i32 as u64)
        }
        TYPE_I64 => {
            let n = primitives::to_f64(value)?;
            Ok(n as i64 as u64)
        }
        TYPE_F32 => {
            let n = primitives::to_f64(value)?;
            Ok((n as f32).to_bits() as u64)
        }
        TYPE_F64 => {
            let n = primitives::to_f64(value)?;
            Ok(n.to_bits())
        }
        _ => Err(JsError::new_string(format!(
            "{}, {} unsupported type",
            func_id,
            arg_idx
        ))),
    }
}

fn build_var_object(realm: &QuickJsRealmAdapter) -> Result<QuickJsValueAdapter, JsError> {
    let var_set = realm.create_function("set", move |realm, _this, args| {
        let var_name = args.first()
            .ok_or_else(|| JsError::new_string("Expected var_name argument".to_string()))?;
        let data = args.get(1)
            .ok_or_else(|| JsError::new_string("Expected data argument".to_string()))?;

        let var_name_string = primitives::to_string_q(realm, var_name)?;

        if data.is_string() {
            let data_string = primitives::to_string_q(realm, data)?;
            var::set(var_name_string, data_string).map_err(|e| JsError::new_string(format!("Var error: {}", e)))?;
        } else if is_array_buffer_q(&data) {
            let bytes = get_array_buffer_buffer_copy_q(realm, data)?;
            var::set(var_name_string, bytes).map_err(|e| JsError::new_string(format!("Var error: {}", e)))?;
        }
        Ok(new_undefined_ref())
    }, 2)?;

    let var_get = realm.create_function("getBytes", move |realm, _this, args| {
        let var_name = args.first()
            .ok_or_else(|| JsError::new_string("Expected var_name argument".to_string()))?;
        let var_name_string = primitives::to_string_q(realm, var_name)?;
        let data = var::get::<Vec<u8>>(var_name_string).map_err(|e| JsError::new_string(format!("Var error: {}", e)))?;
        match data {
            Some(d) => new_uint8_array_copy_q(realm, &d),
            None => Ok(new_null_ref()),
        }
    }, 1)?;

    let var_get_str = realm.create_function("getString", move |realm, _this, args| {
        let var_name = args.first()
            .ok_or_else(|| JsError::new_string("Expected var_name argument".to_string()))?;
        let var_name_string = primitives::to_string_q(realm, var_name)?;
        let data = var::get::<String>(var_name_string).map_err(|e| JsError::new_string(format!("Var error: {}", e)))?;
        match data {
            Some(d) => primitives::from_string_q(realm, &d),
            None => Ok(new_null_ref()),
        }
    }, 1)?;

    let var_object = realm.create_object()?;
    realm.set_object_property(&var_object, "set", &var_set)?;
    realm.set_object_property(&var_object, "getBytes", &var_get)?;
    realm.set_object_property(&var_object, "getString", &var_get_str)?;
    Ok(var_object)
}

fn build_http_object(realm: &QuickJsRealmAdapter) -> Result<QuickJsValueAdapter, JsError> {
    let http_req = realm.create_function("request", move |realm, _this, args| {
        let req = args.first()
            .ok_or_else(|| JsError::new_string("Expected http request argument".to_string()))?
            .clone();

        if !req.is_object() {
            return Err(JsError::new_string("First argument should be an http request object".to_string()));
        }

        let url_prop = realm.get_object_property(&req, "url")?;
        let url = primitives::to_string_q(realm, &url_prop)?;

        let method_prop = realm.get_object_property(&req, "method")?;
        let method_string = if method_prop.is_string() {
            primitives::to_string_q(realm, &method_prop)?
        } else {
            "GET".to_string()
        };

        let mut http_req_builder = HttpRequest::new(url)
            .with_method(method_string);

        let headers_prop = realm.get_object_property(&req, "headers")?;
        if !headers_prop.is_null() && !headers_prop.is_undefined() {
            if !headers_prop.is_object() {
                return Err(JsError::new_string("Expected headers to be an object".to_string()));
            }
            let header_names = realm.get_object_properties(&headers_prop)?;
            for name in header_names {
                let value_prop = realm.get_object_property(&headers_prop, &name)?;
                let value_string = primitives::to_string_q(realm, &value_prop)?;
                http_req_builder.headers.insert(name, value_string);
            }
        }

        let body_arg = args.get(1);
        let http_body = match body_arg {
            None => None,
            Some(body) => {
                if body.is_string() {
                    Some(primitives::to_string_q(realm, body)?)
                } else {
                    None
                }
            }
        };

        let res = http::request(&http_req_builder, http_body).map_err(|e| JsError::new_string(format!("HTTP error: {}", e)))?;
        let body = res.body();
        let body_str = from_utf8(&body).map_err(|e| JsError::new_string(format!("Utf8 error: {}", e)))?;

        let resp_obj = realm.create_object()?;
        realm.set_object_property(&resp_obj, "body", &primitives::from_string_q(realm, body_str)?)?;
        realm.set_object_property(&resp_obj, "status", &primitives::from_i32(res.status_code() as i32))?;

        let headers_obj = realm.create_object()?;
        for (k, v) in res.headers() {
            realm.set_object_property(&headers_obj, k, &primitives::from_string_q(realm, v)?)?;
        }
        realm.set_object_property(&resp_obj, "headers", &headers_obj)?;

        Ok(resp_obj)
    }, 2)?;

    let http_obj = realm.create_object()?;
    realm.set_object_property(&http_obj, "request", &http_req)?;
    Ok(http_obj)
}

fn build_config_object(realm: &QuickJsRealmAdapter) -> Result<QuickJsValueAdapter, JsError> {
    let config_get = realm.create_function("get", move |realm, _this, args| {
        let key = args.first()
            .ok_or_else(|| JsError::new_string("Expected key argument".to_string()))?;
        let key_string = primitives::to_string_q(realm, key)?;

        match config::get(&key_string) {
            Ok(Some(v)) => primitives::from_string_q(realm, &v),
            _ => Ok(new_null_ref()),
        }
    }, 1)?;

    let config_obj = realm.create_object()?;
    realm.set_object_property(&config_obj, "get", &config_get)?;
    Ok(config_obj)
}

fn build_memory(realm: &QuickJsRealmAdapter) -> Result<QuickJsValueAdapter, JsError> {
    let memory_from_buffer = realm.create_function("_fromBuffer", move |realm, _this, args| {
        let data = args.first()
            .ok_or_else(|| JsError::new_string("Expected data argument".to_string()))?;
        if !data.is_object() {
            return Err(JsError::new_string("Expected data to be an object".to_string()));
        }
        if !is_array_buffer_q(&data) {
            return Err(JsError::new_string("Expected data to be an ArrayBuffer".to_string()));
        }
        let bytes = get_array_buffer_buffer_copy_q(realm, data)?;
        let m = extism_pdk::Memory::from_bytes(&bytes).map_err(|e| JsError::new_string(format!("Memory error: {}", e)))?;

        let mem = realm.create_object()?;
        realm.set_object_property(&mem, "offset", &primitives::from_f64(m.offset() as f64))?;
        realm.set_object_property(&mem, "len", &primitives::from_f64(m.len() as f64))?;
        Ok(mem)
    }, 1)?;

    let memory_find = realm.create_function("_find", move |realm, _this, args| {
        let ptr = args.first()
            .ok_or_else(|| JsError::new_string("Expected offset argument".to_string()))?;
        let ptr_val = primitives::to_f64(ptr)? as i64;
        let Some(m) = extism_pdk::Memory::find(ptr_val as u64) else {
            return Ok(new_undefined_ref());
        };
        let mem = realm.create_object()?;
        realm.set_object_property(&mem, "offset", &primitives::from_f64(m.offset() as f64))?;
        realm.set_object_property(&mem, "len", &primitives::from_f64(m.len() as f64))?;
        Ok(mem)
    }, 1)?;

    let memory_free = realm.create_function("_free", move |realm, _this, args| {
        let ptr = args.first()
            .ok_or_else(|| JsError::new_string("Expected offset argument".to_string()))?;
        let ptr_val = primitives::to_f64(ptr)? as i64;
        if let Some(x) = extism_pdk::Memory::find(ptr_val as u64) {
            x.free();
        }
        Ok(new_undefined_ref())
    }, 1)?;

    let read_bytes = realm.create_function("_readBytes", move |realm, _this, args| {
        let ptr = args.first()
            .ok_or_else(|| JsError::new_string("Expected offset argument".to_string()))?;
        let ptr_val = primitives::to_f64(ptr)? as i64;
        let Some(m) = extism_pdk::Memory::find(ptr_val as u64) else {
            return Err(JsError::new_string(format!("Offset did not represent a valid block of memory")));
        };
        let bytes = m.to_vec();
        new_uint8_array_copy_q(realm, &bytes)
    }, 1)?;

    let mem_obj = realm.create_object()?;
    realm.set_object_property(&mem_obj, "_fromBuffer", &memory_from_buffer)?;
    realm.set_object_property(&mem_obj, "_find", &memory_find)?;
    realm.set_object_property(&mem_obj, "_free", &memory_free)?;
    realm.set_object_property(&mem_obj, "_readBytes", &read_bytes)?;
    Ok(mem_obj)
}

fn build_random_bytes(realm: &QuickJsRealmAdapter) -> Result<QuickJsValueAdapter, JsError> {
    realm.create_function("__getRandomBytes", move |realm, _this, args| {
        let n = args.first()
            .ok_or_else(|| JsError::new_string("Expected byte count argument".to_string()))?;
        let n = primitives::to_i32(n)? as usize;
        let mut buf = vec![0u8; n];
        getrandom::getrandom(&mut buf).map_err(|e| JsError::new_string(format!("getrandom failed: {}", e)))?;
        new_uint8_array_copy_q(realm, &buf)
    }, 1)
}

fn build_sha_digest(realm: &QuickJsRealmAdapter) -> Result<QuickJsValueAdapter, JsError> {
    realm.create_function("__shaDigest", move |realm, _this, args| {
        let algo = args.first()
            .ok_or_else(|| JsError::new_string("Expected algorithm name".to_string()))?;
        let algo = primitives::to_string_q(realm, algo)?;

        let data_arg = args.get(1)
            .ok_or_else(|| JsError::new_string("Expected ArrayBuffer data".to_string()))?;
        let bytes = get_array_buffer_buffer_copy_q(realm, data_arg)
            .map_err(|e| JsError::new_string(format!("Expected ArrayBuffer: {}", e)))?;

        let result: Vec<u8> = match algo.as_str() {
            "SHA-1" => {
                let mut hasher = sha1::Sha1::new();
                hasher.update(&bytes);
                hasher.finalize().to_vec()
            }
            "SHA-256" => {
                let mut hasher = sha2::Sha256::new();
                hasher.update(&bytes);
                hasher.finalize().to_vec()
            }
            "SHA-384" => {
                let mut hasher = sha2::Sha384::new();
                hasher.update(&bytes);
                hasher.finalize().to_vec()
            }
            "SHA-512" => {
                let mut hasher = sha2::Sha512::new();
                hasher.update(&bytes);
                hasher.finalize().to_vec()
            }
            _ => return Err(JsError::new_string(format!("Unsupported algorithm: {}", algo))),
        };

        new_uint8_array_copy_q(realm, &result)
    }, 2)
}

fn build_clock(realm: &QuickJsRealmAdapter) -> Result<QuickJsValueAdapter, JsError> {
    realm.create_function("__getTime", move |realm, _this, _args| {
        let now = Utc::now();
        let formatted = now.to_rfc3339_opts(SecondsFormat::Millis, true);
        primitives::from_string_q(realm, &formatted)
    }, 0)
}

fn build_clock_ms(realm: &QuickJsRealmAdapter) -> Result<QuickJsValueAdapter, JsError> {
    realm.create_function("__getTimeMs", move |realm, _this, _args| {
        let now = Utc::now();
        Ok(primitives::from_f64(now.timestamp_millis() as f64))
    }, 0)
}

fn build_decoder(realm: &QuickJsRealmAdapter) -> Result<QuickJsValueAdapter, JsError> {
    realm.create_function("__decodeUtf8BufferToString", move |realm, _this, args| {
        if args.len() != 5 {
            return Err(JsError::new_string(format!("Expecting 5 arguments, received {}", args.len())));
        }

        let js_buffer_value = &args[0];
        let buffer: Vec<u8> = if args[0].is_array() {
            // Array - convert each element to byte
            let len = get_length_q(realm, &args[0])?;
            let mut buf = Vec::with_capacity(len as usize);
            for i in 0..len {
                let elem = realm.get_array_element(&args[0], i as u32)?;
                buf.push(primitives::to_i32(&elem)? as u8);
            }
            buf
        } else {
            get_array_buffer_buffer_copy_q(realm, js_buffer_value)?
        };

        let byte_offset = primitives::to_i32(&args[1])? as usize;
        let byte_length = primitives::to_i32(&args[2])? as usize;
        let fatal = primitives::to_bool(&args[3])?;
        let ignore_bom = primitives::to_bool(&args[4])?;

        let mut view = buffer.get(byte_offset..(byte_offset + byte_length))
            .ok_or_else(|| JsError::new_string("Provided offset and length is not valid for provided buffer".to_string()))?;

        if !ignore_bom {
            view = match view {
                [0xEF, 0xBB, 0xBF, rest @ ..] => rest,
                _ => view,
            };
        }

        let str = if fatal {
            std::borrow::Cow::from(from_utf8(view).map_err(|e| JsError::new_string(format!("Utf8 error: {}", e)))?)
        } else {
            String::from_utf8_lossy(view)
        };

        primitives::from_string_q(realm, &str)
    }, 5)
}

fn build_encoder(realm: &QuickJsRealmAdapter) -> Result<QuickJsValueAdapter, JsError> {
    realm.create_function("__encodeStringToUtf8Buffer", move |realm, _this, args| {
        if args.len() != 1 {
            return Err(JsError::new_string(format!("Expecting 1 argument, got {}", args.len())));
        }

        let js_string = &args[0];
        let rust_string = primitives::to_string_q(realm, js_string)?;
        let buffer = rust_string.as_bytes();
        new_uint8_array_copy_q(realm, buffer)
    }, 1)
}