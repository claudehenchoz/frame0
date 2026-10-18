//! Direct2D 1.1 + DXGI + D3D11 substrate.
//!
//! Holds the long-lived factories and device context. Per-window state
//! (swap chain + render target bitmap) is created via [`D2D::attach_to_hwnd`]
//! and lives on the editor canvas.

use crate::error::{Error, Result};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_OPTIONS,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, ID2D1Bitmap1, ID2D1Device, ID2D1DeviceContext,
    ID2D1Factory1,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_10_0,
    D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, ID3D11Device,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1,
};
use windows::core::Interface;

pub struct D2D {
    pub d3d_device: ID3D11Device,
    pub dxgi_device: IDXGIDevice,
    pub dxgi_factory: IDXGIFactory2,
    pub factory: ID2D1Factory1,
    pub device: ID2D1Device,
    pub context: ID2D1DeviceContext,
}

impl D2D {
    pub fn create() -> Result<Self> {
        unsafe {
            let d3d_device = create_d3d11()?;
            let dxgi_device: IDXGIDevice = d3d_device.cast()?;
            let dxgi_adapter = dxgi_device.GetAdapter()?;
            let dxgi_factory: IDXGIFactory2 = dxgi_adapter.GetParent()?;

            let opts = D2D1_FACTORY_OPTIONS::default();
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&opts))
                    ?;
            let device = factory.CreateDevice(&dxgi_device)?;
            let context = device
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
                ?;

            Ok(Self {
                d3d_device,
                dxgi_device,
                dxgi_factory,
                factory,
                device,
                context,
            })
        }
    }

    /// Create a flip-model swap chain bound to `hwnd` and a D2D render-target
    /// bitmap pointing at its back buffer.
    pub fn create_swapchain_target(
        &self,
        hwnd: HWND,
        width: u32,
        height: u32,
    ) -> Result<(IDXGISwapChain1, ID2D1Bitmap1)> {
        unsafe {
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width.max(1),
                Height: height.max(1),
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: windows::Win32::Graphics::Dxgi::Common::DXGI_ALPHA_MODE_IGNORE,
                Flags: 0,
            };
            let swap = self
                .dxgi_factory
                .CreateSwapChainForHwnd(&self.d3d_device, hwnd, &desc, None, None)
                ?;
            let target = self.target_bitmap_for_swap(&swap)?;
            Ok((swap, target))
        }
    }

    /// Get a D2D render-target bitmap pointing at the swap chain's current back buffer.
    pub fn target_bitmap_for_swap(&self, swap: &IDXGISwapChain1) -> Result<ID2D1Bitmap1> {
        unsafe {
            let surface: IDXGISurface = swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: core::mem::ManuallyDrop::new(None),
            };
            Ok(self
                .context
                .CreateBitmapFromDxgiSurface(&surface, Some(&props))?)
        }
    }

    pub fn present(&self, swap: &IDXGISwapChain1) {
        unsafe {
            let _ = swap.Present(1, DXGI_PRESENT(0));
        }
    }
}

unsafe fn create_d3d11() -> Result<ID3D11Device> {
    let levels = [D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_0];
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device: Option<ID3D11Device> = None;
        let hr = unsafe {
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                None,
            )
        };
        if hr.is_ok() {
            if let Some(d) = device {
                return Ok(d);
            }
        }
    }
    Err(Error::msg("D3D11CreateDevice failed"))
}

pub fn color_f(c: crate::gfx::color::Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c.r as f32 / 255.0,
        g: c.g as f32 / 255.0,
        b: c.b as f32 / 255.0,
        a: c.a as f32 / 255.0,
    }
}

/// Upload an in-memory straight-alpha BGRA bitmap as an ID2D1Bitmap1
/// (immutable, drawable). Pixels are premultiplied on the way in when the
/// image has any transparency.
pub fn upload_bitmap(
    ctx: &ID2D1DeviceContext,
    bm: &crate::gfx::bitmap::Bitmap,
) -> Result<ID2D1Bitmap1> {
    let opaque = bm.bgra.chunks_exact(4).all(|p| p[3] == 255);
    let premul;
    let data: &[u8] = if opaque {
        &bm.bgra
    } else {
        premul = bm
            .bgra
            .chunks_exact(4)
            .flat_map(|p| {
                let a = p[3] as u32;
                [
                    (p[0] as u32 * a / 255) as u8,
                    (p[1] as u32 * a / 255) as u8,
                    (p[2] as u32 * a / 255) as u8,
                    p[3],
                ]
            })
            .collect::<Vec<u8>>();
        &premul
    };
    unsafe {
        let size = windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U {
            width: bm.width.max(1),
            height: bm.height.max(1),
        };
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: windows::Win32::Graphics::Direct2D::D2D1_BITMAP_OPTIONS_NONE,
            colorContext: core::mem::ManuallyDrop::new(None),
        };
        let pitch = bm.width * 4;
        Ok(ctx.CreateBitmap(
            size,
            Some(data.as_ptr() as *const _),
            pitch,
            &props,
        )?)
    }
}
