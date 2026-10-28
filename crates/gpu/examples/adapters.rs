//! Diagnostics: which adapters does this machine have, on which backends, and which one does the
//! **export renderer** pick? `adapters`.
//!
//! The export renderer creates its own `wgpu::Instance::default()` and takes the default adapter
//! (`crates/gpu/src/export_renderer.rs`), while the app deliberately asks for `Backends::PRIMARY`
//! — Vulkan and DX12 both — because "forcing DX12 alone sends shaders through FXC (which rejects
//! `fx.wgsl`, silently falling back to the CPU)" (`apps/filmcraft/src/graphics.rs`). On a machine
//! that has a Vulkan adapter for the same GPU the two agree and nothing is wrong; on one that
//! does not, the export renderer takes DX12 and the export renders on the CPU with nothing on
//! screen saying so (#1616).
//!
//! This prints both answers, so the difference is visible on the machine it happens on.

fn main() {
    println!("enabled backend features: {:?}", wgpu::Instance::enabled_backend_features());

    let instance = wgpu::Instance::default();
    for a in pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all())) {
        let i = a.get_info();
        println!("adapter: {:?} backend {:?} driver {:?}", i.name, i.backend, i.driver);
    }

    // what the export renderer asks for
    match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) {
        Ok(a) => {
            let i = a.get_info();
            println!("default request_adapter picks: {:?} on {:?}", i.name, i.backend);
        }
        Err(e) => println!("default request_adapter picks: nothing ({e})"),
    }

    // and what the app asks for (Backends::PRIMARY, both Vulkan and DX12)
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::PRIMARY;
    let primary = wgpu::Instance::new(desc);
    match pollster::block_on(primary.request_adapter(&wgpu::RequestAdapterOptions::default())) {
        Ok(a) => {
            let i = a.get_info();
            println!("PRIMARY request_adapter picks: {:?} on {:?}", i.name, i.backend);
        }
        Err(e) => println!("PRIMARY request_adapter picks: nothing ({e})"),
    }
}
