/*
 * velocityrl
 * Copyright (c) 2026 bits (https://github.com/bitsfdb/velocityrl)
 * 
 * Licensed under the GNU General Public License v3.0.
 * unauthorized rebranding or stripping of this copyright notice is strictly prohibited.
 */

use rquickjs::{Context, Function, Object, Runtime};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const DEFAULT_MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;
pub const DEFAULT_MAX_STACK_BYTES: usize = 1024 * 1024;
pub const MAX_SCRIPT_EXEC_SECS: u64 = 5;

#[derive(Clone)]
pub struct HostContext {
    pub game_dir: PathBuf,
}

pub struct PluginEngine {
    #[allow(dead_code)]
    runtime: Runtime,
    context: Context,
    host: HostContext,
}

impl PluginEngine {
    pub fn new(game_dir: PathBuf) -> Result<Self, String> {
        let runtime = Runtime::new().map_err(|e| format!("Failed to create QuickJS runtime: {e}"))?;

        runtime.set_memory_limit(DEFAULT_MAX_MEMORY_BYTES);
        runtime.set_max_stack_size(DEFAULT_MAX_STACK_BYTES);

        let start_time = Arc::new(Mutex::new(Instant::now()));
        let interrupt_timer = Arc::clone(&start_time);
        runtime.set_interrupt_handler(Some(Box::new(move || {
            if let Ok(st) = interrupt_timer.lock() {
                st.elapsed().as_secs() > MAX_SCRIPT_EXEC_SECS
            } else {
                false
            }
        })));

        let context = Context::full(&runtime)
            .map_err(|e| format!("Failed to create QuickJS context: {e}"))?;

        let host = HostContext { game_dir };
        let engine = Self { runtime, context, host };
        engine.register_host_api()?;

        Ok(engine)
    }

    fn register_host_api(&self) -> Result<(), String> {
        let host_ctx = self.host.clone();

        self.context.with(|ctx| {
            let globals = ctx.globals();

            let console = Object::new(ctx.clone()).unwrap();
            console.set("log", Function::new(ctx.clone(), |msg: String| {
                crate::applog::event(&format!("[RLIM-JS] {msg}"));
            }).unwrap()).unwrap();
            console.set("info", Function::new(ctx.clone(), |msg: String| {
                crate::applog::event(&format!("[RLIM-JS:INFO] {msg}"));
            }).unwrap()).unwrap();
            console.set("warn", Function::new(ctx.clone(), |msg: String| {
                crate::applog::event(&format!("[RLIM-JS:WARN] {msg}"));
            }).unwrap()).unwrap();
            console.set("error", Function::new(ctx.clone(), |msg: String| {
                crate::applog::event(&format!("[RLIM-JS:ERROR] {msg}"));
            }).unwrap()).unwrap();
            globals.set("console", console).unwrap();

            let rlim = Object::new(ctx.clone()).unwrap();
            let patcher = Object::new(ctx.clone()).unwrap();

            let _host_c = host_ctx.clone();
            let patch_fn = Function::new(ctx.clone(), move |package: String, class_name: String, func_name: String, bytecode: Vec<u8>| -> bool {
                crate::applog::event(&format!(
                    "[RLIM-JS:Patcher] Patching {}::{}.{} ({} bytes injected)",
                    package, class_name, func_name, bytecode.len()
                ));
                true
            }).unwrap();
            patcher.set("patchFunction", patch_fn).unwrap();

            let _host_c2 = host_ctx.clone();
            let create_fn = Function::new(ctx.clone(), move |package: String, class_name: String, func_name: String, flags: u32, bytecode: Vec<u8>| -> bool {
                crate::applog::event(&format!(
                    "[RLIM-JS:Patcher] Creating custom function {}::{}.{} [flags=0x{:08X}] ({} bytes)",
                    package, class_name, func_name, flags, bytecode.len()
                ));
                true
            }).unwrap();
            patcher.set("createFunction", create_fn).unwrap();

            let neutralize_fn = Function::new(ctx.clone(), move |package: String, class_name: String, func_name: String, ret: bool| -> bool {
                crate::applog::event(&format!(
                    "[RLIM-JS:Patcher] Neutralizing {}::{}.{} (return={})",
                    package, class_name, func_name, ret
                ));
                true
            }).unwrap();
            patcher.set("neutralizeFunction", neutralize_fn).unwrap();

            rlim.set("patcher", patcher).unwrap();

            let shaders = Object::new(ctx.clone()).unwrap();

            let set_camber_fn = Function::new(ctx.clone(), move |front: f32, rear: f32| -> bool {
                crate::applog::event(&format!("[RLIM-JS:Shaders] Stance set: front={front}°, rear={rear}°"));
                true
            }).unwrap();
            shaders.set("setCamber", set_camber_fn).unwrap();

            let set_underglow_fn = Function::new(ctx.clone(), move |r: f32, g: f32, b: f32, brightness: f32, mode: u32| -> bool {
                crate::applog::event(&format!("[RLIM-JS:Shaders] Underglow: rgb=({r}, {g}, {b}) brightness={brightness} mode={mode}"));
                true
            }).unwrap();
            shaders.set("setUnderglow", set_underglow_fn).unwrap();

            rlim.set("shaders", shaders).unwrap();

            let spawner = Object::new(ctx.clone()).unwrap();
            let spawn_fn = Function::new(ctx.clone(), move |product_id: i32, paint_id: i32| -> bool {
                crate::applog::event(&format!("[RLIM-JS:Spawner] Spawning Product ID {product_id} with paint {paint_id}"));
                true
            }).unwrap();
            spawner.set("spawnItem", spawn_fn).unwrap();
            rlim.set("spawner", spawner).unwrap();

            let config = Object::new(ctx.clone()).unwrap();
            let get_cfg_fn = Function::new(ctx.clone(), move |_key: String, default_val: String| -> String {
                default_val
            }).unwrap();
            let set_cfg_fn = Function::new(ctx.clone(), move |key: String, val: String| -> bool {
                crate::applog::event(&format!("[RLIM-JS:Config] {key} = {val}"));
                true
            }).unwrap();
            config.set("get", get_cfg_fn).unwrap();
            config.set("set", set_cfg_fn).unwrap();
            rlim.set("config", config).unwrap();

            globals.set("rlim", rlim).unwrap();

            Ok(())
        }).map_err(|e: rquickjs::Error| format!("Failed to register host API in QuickJS: {e}"))
    }

    pub fn eval_script(&self, script: &str) -> Result<(), String> {
        self.context.with(|ctx| {
            ctx.eval::<(), _>(script)
                .map_err(|e| format!("QuickJS runtime error: {e}"))
        })
    }

    pub fn load_plugin_file(&self, path: &Path) -> Result<(), String> {
        let code = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read plugin file {}: {e}", path.display()))?;
        crate::applog::event(&format!("Executing JS plugin: {}", path.display()));
        self.eval_script(&code)
    }
}
