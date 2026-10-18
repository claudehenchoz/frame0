// Frame Zero - a tiny Windows screenshot and annotation tool
// Copyright (C) 2026 Claude Henchoz
// SPDX-License-Identifier: GPL-3.0-or-later
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Direct2D 1.1 + DXGI + D3D11 substrate.
//!
//! Holds the long-lived factories and device context. Per-window state
//! (a GDI-compatible render target bitmap) is created via
//! [`D2D::create_gdi_target`] and lives on the editor canvas.

use crate::error::{Error, Result};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
    D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, D2D1_BITMAP_OPTIONS_GDI_COMPATIBLE, D2D1_BITMAP_OPTIONS_TARGET,
    D2D1_BITMAP_PROPERTIES1, D2D1_DC_INITIALIZE_MODE_COPY, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
    D2D1_FACTORY_OPTIONS, D2D1_FACTORY_TYPE_SINGLE_THREADED, ID2D1Bitmap1, ID2D1Device,
    ID2D1DeviceContext, ID2D1Factory1, ID2D1GdiInteropRenderTarget,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP, D3D_FEATURE_LEVEL_10_0,
    D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, ID3D11Device,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::core::Interface;

pub struct D2D {
    pub d3d_device: ID3D11Device,
    pub dxgi_device: IDXGIDevice,
    pub factory: ID2D1Factory1,
    pub device: ID2D1Device,
    pub context: ID2D1DeviceContext,
}

impl D2D {
    pub fn create() -> Result<Self> {
        unsafe {
            let d3d_device = create_d3d11()?;
            let dxgi_device: IDXGIDevice = d3d_device.cast()?;

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
                factory,
                device,
                context,
            })
        }
    }

    /// Create a GDI-compatible offscreen render target. Windows paint it with
    /// `blit` instead of presenting a DXGI swap chain: D3D presents drive
    /// windowed G-SYNC / VRR, so the display (and the mouse cursor) drops to
    /// the app's frame rate for about a second whenever it stops drawing.
    pub fn create_gdi_target(&self, width: u32, height: u32) -> Result<ID2D1Bitmap1> {
        unsafe {
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_GDI_COMPATIBLE,
                colorContext: core::mem::ManuallyDrop::new(None),
            };
            let size = D2D_SIZE_U { width: width.max(1), height: height.max(1) };
            Ok(self.context.CreateBitmap(size, None, 0, &props)?)
        }
    }

    /// Copy the GDI-compatible target being drawn into `hdc`. Call between
    /// `BeginDraw` and `EndDraw` (GDI interop requires it).
    pub fn blit(&self, hdc: isize, width: u32, height: u32) -> Result<()> {
        unsafe {
            let gdi: ID2D1GdiInteropRenderTarget = self.context.cast()?;
            let src = gdi.GetDC(D2D1_DC_INITIALIZE_MODE_COPY)?;
            windows_sys::Win32::Graphics::Gdi::BitBlt(
                hdc,
                0,
                0,
                width as i32,
                height as i32,
                src.0 as isize,
                0,
                0,
                windows_sys::Win32::Graphics::Gdi::SRCCOPY,
            );
            gdi.ReleaseDC(None)?;
        }
        Ok(())
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
