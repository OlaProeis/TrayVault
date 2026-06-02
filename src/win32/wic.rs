//! Windows Imaging Component (WIC) encode/decode for BGRA clipboard pixels.
//!
//! Uses hand-declared COM vtables against `windowscodecs.dll` and `ole32.dll`.
//! Call [`ensure_com_initialized`] once per worker thread before encode/decode.

#![allow(
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    clippy::upper_case_acronyms
)]

use core::ffi::c_void;
use core::mem::MaybeUninit;
use core::ptr::{null, null_mut};

use crate::config::ImageBlobCodec;
use crate::error::{ClipError, Result};
use crate::log;
use crate::win32::ffi::{
    self, succeeded, CoCreateInstance, CoInitializeEx, CreateStreamOnHGlobal, GetHGlobalFromStream,
    GetProcAddress, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, LoadLibraryW, VariantClear,
    VariantInit, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, GHND, GUID, HRESULT, LPVOID,
    PROPBAG2, RPC_E_CHANGED_MODE, VARIANT, VT_R4,
};
use crate::win32::wide;

// ---------------------------------------------------------------------------
// GUIDs (CLSID / IID / pixel / container formats)
// ---------------------------------------------------------------------------

const CLSID_WICImagingFactory: GUID = GUID {
    data1: 0xcacaf262,
    data2: 0x9370,
    data3: 0x4615,
    data4: [0xa1, 0x3b, 0x9f, 0x55, 0x39, 0xda, 0x4c, 0x0a],
};

/// Legacy WIC factory CLSID (pre–Windows 10 1903); kept as fallback.
const CLSID_WICImagingFactory_Legacy: GUID = GUID {
    data1: 0xcacaf262,
    data2: 0x9370,
    data3: 0x4615,
    data4: [0xa1, 0x3b, 0x9f, 0x55, 0x3f, 0xfc, 0x96, 0x21],
};

const IID_IWICImagingFactory: GUID = GUID {
    data1: 0xec5ec8a9,
    data2: 0xc395,
    data3: 0x4314,
    data4: [0x9c, 0x77, 0x54, 0xd7, 0xa9, 0x35, 0xff, 0x70],
};

const IID_IClassFactory: GUID = GUID {
    data1: 0x00000001,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

const GUID_WICPixelFormat32bppBGRA: GUID = GUID {
    data1: 0x6fdddc32,
    data2: 0x4e03,
    data3: 0x4bfe,
    data4: [0xb1, 0x85, 0x3d, 0x77, 0x76, 0xdc, 0x50, 0x77],
};

const GUID_ContainerFormatPng: GUID = GUID {
    data1: 0x116cfdc6,
    data2: 0x4cc4,
    data3: 0x4288,
    data4: [0x91, 0x7b, 0x09, 0x91, 0x75, 0x55, 0x3a, 0xd1],
};

const GUID_ContainerFormatJpeg: GUID = GUID {
    data1: 0x19e4a5aa,
    data2: 0x5662,
    data3: 0x4fc5,
    data4: [0xa0, 0xc0, 0x17, 0xd5, 0x58, 0x35, 0x8b, 0xc8],
};

const IID_IWICBitmapEncoder: GUID = GUID {
    data1: 0x23e976ea,
    data2: 0x1e1d,
    data3: 0x4564,
    data4: [0xa1, 0x80, 0x2d, 0xfb, 0x54, 0x67, 0xfb, 0x48],
};

const IID_IWICBitmapFrameEncode: GUID = GUID {
    data1: 0x00000102,
    data2: 0xa8f7,
    data3: 0x4353,
    data4: [0x83, 0x39, 0x94, 0xb2, 0x73, 0xfb, 0x44, 0x37],
};

const IID_IWICBitmapDecoder: GUID = GUID {
    data1: 0x9edde9e7,
    data2: 0x665d,
    data3: 0x4a87,
    data4: [0x8c, 0xe3, 0x86, 0x78, 0xf6, 0xb3, 0xe4, 0x89],
};

const IID_IWICBitmapFrameDecode: GUID = GUID {
    data1: 0x42191802,
    data2: 0x6b89,
    data3: 0x413f,
    data4: [0x9c, 0x5b, 0x5b, 0x0d, 0x9f, 0x7e, 0xf6, 0xaf],
};

const IID_IWICFormatConverter: GUID = GUID {
    data1: 0xa1dc703a,
    data2: 0x661c,
    data3: 0x4ac4,
    data4: [0x84, 0x94, 0x88, 0x5d, 0xc4, 0x89, 0x68, 0x2a],
};

const IID_IPropertyBag2: GUID = GUID {
    data1: 0x22f55882,
    data2: 0x280a,
    data3: 0x11d0,
    data4: [0xa8, 0xa9, 0x00, 0xa0, 0xc9, 0x0c, 0x20, 0x04],
};

// WIC constants not yet in ffi.rs
const WICBitmapCacheOnDemand: u32 = 0x1;
const WICBitmapNoCache: u32 = 0x0;
const WICBitmapEncoderNoCache: u32 = 0x2;
const WICBitmapEncoderCacheInMemory: u32 = 0x0;
const WICBitmapLockWrite: u32 = 0x1;
const WICDecodeMetadataCacheOnLoad: u32 = 0x1;

// ---------------------------------------------------------------------------
// COM vtables (minimal surface for blob encode/decode)
// ---------------------------------------------------------------------------

#[repr(C)]
struct IUnknown {
    vtable: *const IUnknownVtbl,
}

type QueryInterfaceFn =
    unsafe extern "system" fn(*mut IUnknown, *const GUID, *mut LPVOID) -> HRESULT;
type AddRefFn = unsafe extern "system" fn(*mut IUnknown) -> u32;
type ReleaseFn = unsafe extern "system" fn(*mut IUnknown) -> u32;

#[repr(C)]
struct IUnknownVtbl {
    QueryInterface: QueryInterfaceFn,
    AddRef: AddRefFn,
    Release: ReleaseFn,
}

#[repr(C)]
struct IClassFactory {
    vtable: *const IClassFactoryVtbl,
}

#[repr(C)]
struct IClassFactoryVtbl {
    unknown: IUnknownVtbl,
    CreateInstance:
        unsafe extern "system" fn(*mut IClassFactory, LPVOID, *const GUID, *mut LPVOID) -> HRESULT,
    LockServer: unsafe extern "system" fn(*mut IClassFactory, i32) -> HRESULT,
}

type DllGetClassObjectFn =
    unsafe extern "system" fn(*const GUID, *const GUID, *mut LPVOID) -> HRESULT;

const REGDB_E_CLASSNOTREG: HRESULT = 0x8004_0154_u32 as i32;

#[repr(C)]
struct IWICImagingFactory {
    vtable: *const IWICImagingFactoryVtbl,
}

#[repr(C)]
struct IWICImagingFactoryVtbl {
    unknown: IUnknownVtbl,
    CreateDecoderFromFilename: *const c_void,
    CreateDecoderFromStream: unsafe extern "system" fn(
        *mut IWICImagingFactory,
        LPVOID,
        *const GUID,
        u32,
        *mut LPVOID,
    ) -> HRESULT,
    CreateDecoderFromFileHandle: *const c_void,
    CreateComponentInfo: *const c_void,
    CreateDecoder: *const c_void,
    CreateEncoder: unsafe extern "system" fn(
        *mut IWICImagingFactory,
        *const GUID,
        *const GUID,
        *mut LPVOID,
    ) -> HRESULT,
    CreatePalette: *const c_void,
    CreateFormatConverter:
        unsafe extern "system" fn(*mut IWICImagingFactory, *mut LPVOID) -> HRESULT,
    CreateBitmapScaler: *const c_void,
    CreateBitmapClipper: *const c_void,
    CreateBitmapFlipRotator: *const c_void,
    CreateStream: *const c_void,
    CreateColorContext: *const c_void,
    CreateColorTransformer: *const c_void,
    CreateBitmap: *const c_void,
    CreateBitmapFromSource: *const c_void,
    CreateBitmapFromSourceRect: *const c_void,
    CreateBitmapFromMemory: *const c_void,
    CreateBitmapFromHBITMAP: *const c_void,
    CreateBitmapFromHICON: *const c_void,
    CreateComponentEnumerator: *const c_void,
    CreateFastMetadataEncoderFromDecoder: *const c_void,
    CreateFastMetadataEncoderFromFrameDecode: *const c_void,
    CreateQueryWriter: *const c_void,
    CreateQueryWriterFromReader: *const c_void,
}

#[repr(C)]
struct IWICBitmap {
    vtable: *const IWICBitmapVtbl,
}

#[repr(C)]
struct IWICBitmapVtbl {
    unknown: IUnknownVtbl,
    GetSize: *const c_void,
    GetPixelFormat: *const c_void,
    GetResolution: *const c_void,
    CopyPalette: *const c_void,
    CopyPixels: *const c_void,
    Lock: unsafe extern "system" fn(*mut IWICBitmap, *const WICRect, u32, *mut LPVOID) -> HRESULT,
}

#[repr(C)]
struct IWICBitmapLock {
    vtable: *const IWICBitmapLockVtbl,
}

#[repr(C)]
struct IWICBitmapLockVtbl {
    unknown: IUnknownVtbl,
    GetSize: *const c_void,
    GetStride: unsafe extern "system" fn(*mut IWICBitmapLock, *mut u32) -> HRESULT,
    GetDataPointer:
        unsafe extern "system" fn(*mut IWICBitmapLock, *mut u32, *mut *mut u8) -> HRESULT,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct WICRect {
    X: i32,
    Y: i32,
    Width: u32,
    Height: u32,
}

#[repr(C)]
struct IWICBitmapEncoder {
    vtable: *const IWICBitmapEncoderVtbl,
}

#[repr(C)]
struct IWICBitmapEncoderVtbl {
    unknown: IUnknownVtbl,
    Initialize: unsafe extern "system" fn(*mut IWICBitmapEncoder, LPVOID, u32) -> HRESULT,
    GetContainerFormat: *const c_void,
    GetEncoderInfo: *const c_void,
    SetPalette: *const c_void,
    SetColorContext: *const c_void,
    SetThumbnail: *const c_void,
    SetPreview: *const c_void,
    CreateNewFrame:
        unsafe extern "system" fn(*mut IWICBitmapEncoder, *mut LPVOID, *mut LPVOID) -> HRESULT,
    Commit: unsafe extern "system" fn(*mut IWICBitmapEncoder) -> HRESULT,
}

#[repr(C)]
struct IWICBitmapFrameEncode {
    vtable: *const IWICBitmapFrameEncodeVtbl,
}

#[repr(C)]
struct IWICBitmapFrameEncodeVtbl {
    unknown: IUnknownVtbl,
    Initialize: unsafe extern "system" fn(*mut IWICBitmapFrameEncode, LPVOID) -> HRESULT,
    SetSize: unsafe extern "system" fn(*mut IWICBitmapFrameEncode, u32, u32) -> HRESULT,
    SetResolution: *const c_void,
    SetPixelFormat: unsafe extern "system" fn(*mut IWICBitmapFrameEncode, *mut GUID) -> HRESULT,
    SetColorContexts: *const c_void,
    SetThumbnail: *const c_void,
    SetProperties: *const c_void,
    WritePixels:
        unsafe extern "system" fn(*mut IWICBitmapFrameEncode, u32, u32, u32, *const u8) -> HRESULT,
    WriteSource:
        unsafe extern "system" fn(*mut IWICBitmapFrameEncode, LPVOID, *const WICRect) -> HRESULT,
    Commit: unsafe extern "system" fn(*mut IWICBitmapFrameEncode) -> HRESULT,
}

#[repr(C)]
struct IPropertyBag2 {
    vtable: *const IPropertyBag2Vtbl,
}

#[repr(C)]
struct IPropertyBag2Vtbl {
    unknown: IUnknownVtbl,
    Read: *const c_void,
    Write: unsafe extern "system" fn(
        *mut IPropertyBag2,
        u32,
        *const PROPBAG2,
        *mut VARIANT,
    ) -> HRESULT,
}

#[repr(C)]
struct IWICBitmapDecoder {
    vtable: *const IWICBitmapDecoderVtbl,
}

#[repr(C)]
struct IWICBitmapDecoderVtbl {
    unknown: IUnknownVtbl,
    QueryCapability: *const c_void,
    Initialize: *const c_void,
    GetContainerFormat: *const c_void,
    GetDecoderInfo: *const c_void,
    CopyPalette: *const c_void,
    GetMetadataReader: *const c_void,
    GetPreview: *const c_void,
    CreateColorContext: *const c_void,
    GetThumbnail: *const c_void,
    GetFrameCount: unsafe extern "system" fn(*mut IWICBitmapDecoder, *mut u32) -> HRESULT,
    GetFrame: unsafe extern "system" fn(*mut IWICBitmapDecoder, u32, *mut LPVOID) -> HRESULT,
}

#[repr(C)]
struct IWICBitmapFrameDecode {
    vtable: *const IWICBitmapFrameDecodeVtbl,
}

#[repr(C)]
struct IWICBitmapFrameDecodeVtbl {
    unknown: IUnknownVtbl,
    GetSize: unsafe extern "system" fn(*mut IWICBitmapFrameDecode, *mut u32, *mut u32) -> HRESULT,
    GetPixelFormat: *const c_void,
    GetResolution: *const c_void,
    CopyPalette: *const c_void,
    GetColorContexts: *const c_void,
    GetThumbnail: *const c_void,
    GetMetadataReader: *const c_void,
    CopyPixels: *const c_void,
}

#[repr(C)]
struct IWICFormatConverter {
    vtable: *const IWICFormatConverterVtbl,
}

#[repr(C)]
struct IWICFormatConverterVtbl {
    unknown: IUnknownVtbl,
    Initialize: unsafe extern "system" fn(
        *mut IWICFormatConverter,
        LPVOID,
        *const GUID,
        u32,
        *const GUID,
        f32,
        u32,
    ) -> HRESULT,
    CanConvert: *const c_void,
    CopyPixels: unsafe extern "system" fn(
        *mut IWICFormatConverter,
        *const WICRect,
        u32,
        u32,
        *mut u8,
    ) -> HRESULT,
}

// ---------------------------------------------------------------------------
// COM helpers
// ---------------------------------------------------------------------------

struct ComPtr<T> {
    ptr: *mut T,
}

impl<T> ComPtr<T> {
    fn from_raw(ptr: *mut T) -> Self {
        Self { ptr }
    }

    fn as_ptr(&self) -> *mut T {
        self.ptr
    }

    fn as_unknown(&self) -> *mut IUnknown {
        self.ptr.cast()
    }

    fn release(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: COM Release via vtable.
            unsafe {
                let vtable = (*self.as_unknown()).vtable;
                ((*vtable).Release)(self.as_unknown());
            }
            self.ptr = null_mut();
        }
    }
}

impl<T> Drop for ComPtr<T> {
    fn drop(&mut self) {
        self.release();
    }
}

fn hr_err(api: &'static str, hr: HRESULT) -> ClipError {
    ClipError::Other(format!("{api} failed (HRESULT 0x{hr:08X})"))
}

fn check_hr(api: &'static str, hr: HRESULT) -> Result<()> {
    if succeeded(hr) {
        Ok(())
    } else {
        Err(hr_err(api, hr))
    }
}

thread_local! {
    static COM_INITIALIZED: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// Initialize COM on the calling thread (idempotent).
pub fn ensure_com_initialized() -> Result<()> {
    COM_INITIALIZED.with(|flag| {
        if flag.get() {
            return Ok(());
        }
        // SAFETY: per-thread COM init for WIC.
        let hr = unsafe { CoInitializeEx(null_mut(), COINIT_APARTMENTTHREADED) };
        if succeeded(hr) || hr == RPC_E_CHANGED_MODE {
            flag.set(true);
            Ok(())
        } else {
            Err(hr_err("CoInitializeEx", hr))
        }
    })
}

fn create_factory() -> Result<ComPtr<IWICImagingFactory>> {
    ensure_com_initialized()?;
    for clsid in [CLSID_WICImagingFactory, CLSID_WICImagingFactory_Legacy] {
        if let Ok(factory) = create_factory_with_clsid(clsid) {
            return Ok(factory);
        }
    }
    create_factory_from_dll(CLSID_WICImagingFactory)
        .or_else(|_| create_factory_from_dll(CLSID_WICImagingFactory_Legacy))
}

fn create_factory_with_clsid(clsid: GUID) -> Result<ComPtr<IWICImagingFactory>> {
    let mut factory: LPVOID = null_mut();
    // SAFETY: CoCreateInstance for WIC factory (registry path).
    let hr = unsafe {
        CoCreateInstance(
            &clsid,
            null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_IWICImagingFactory,
            &mut factory,
        )
    };
    check_hr("CoCreateInstance(WICImagingFactory)", hr)?;
    Ok(ComPtr::from_raw(factory.cast()))
}

fn create_factory_from_dll(clsid: GUID) -> Result<ComPtr<IWICImagingFactory>> {
    let wic_dll = wide("windowscodecs.dll");
    // SAFETY: load WIC DLL and resolve DllGetClassObject (unregistered CLSID fallback).
    unsafe {
        let module = LoadLibraryW(wic_dll.as_ptr());
        if module == 0 {
            return Err(ClipError::Other(
                "LoadLibraryW(windowscodecs.dll) failed".into(),
            ));
        }
        let proc = GetProcAddress(module, c"DllGetClassObject".as_ptr().cast());
        if proc.is_null() {
            return Err(ClipError::Other(
                "GetProcAddress(DllGetClassObject) failed".into(),
            ));
        }
        let get_class_object: DllGetClassObjectFn = core::mem::transmute(proc);
        let mut class_factory: LPVOID = null_mut();
        let hr = get_class_object(&clsid, &IID_IClassFactory, &mut class_factory);
        check_hr("DllGetClassObject(WICImagingFactory)", hr)?;

        let cf = class_factory as *mut IClassFactory;
        let mut factory: LPVOID = null_mut();
        let hr =
            ((*(*cf).vtable).CreateInstance)(cf, null_mut(), &IID_IWICImagingFactory, &mut factory);
        release_unknown(class_factory);
        check_hr("IClassFactory::CreateInstance(WICImagingFactory)", hr)?;
        Ok(ComPtr::from_raw(factory.cast()))
    }
}

unsafe fn release_unknown(ptr: LPVOID) {
    if ptr.is_null() {
        return;
    }
    let unk = ptr as *mut IUnknown;
    let vtable = (*unk).vtable;
    ((*vtable).Release)(unk);
}

// ---------------------------------------------------------------------------
// Public encode / decode API
// ---------------------------------------------------------------------------

pub fn wic_codecs_available() -> bool {
    let Ok(factory) = create_factory() else {
        return false;
    };
    let mut encoder: LPVOID = null_mut();
    // SAFETY: probe whether PNG encoder can be instantiated on this OS install.
    unsafe {
        let vtbl = (*factory.as_ptr()).vtable;
        let hr = ((*vtbl).CreateEncoder)(
            factory.as_ptr(),
            &GUID_ContainerFormatPng,
            null(),
            &mut encoder,
        );
        if succeeded(hr) && !encoder.is_null() {
            release_unknown(encoder);
            true
        } else {
            false
        }
    }
}

/// Encode top-down BGRA pixels to PNG or JPEG bytes via WIC.
pub fn encode_bgra(
    width: u32,
    height: u32,
    pixels: &[u8],
    codec: ImageBlobCodec,
    jpeg_quality: u8,
) -> Result<Vec<u8>> {
    let expected = (width as u64)
        .checked_mul(height as u64)
        .and_then(|n| n.checked_mul(4));
    let Some(expected) = expected else {
        return Err(ClipError::Other("image dimensions overflow".into()));
    };
    if pixels.len() as u64 != expected {
        return Err(ClipError::Other(format!(
            "pixel buffer length {} does not match {}x{} BGRA",
            pixels.len(),
            width,
            height
        )));
    }

    let factory = create_factory()?;

    let container = match codec {
        ImageBlobCodec::Png => &GUID_ContainerFormatPng,
        ImageBlobCodec::Jpeg => &GUID_ContainerFormatJpeg,
    };

    let mut stream: LPVOID = null_mut();
    // SAFETY: in-memory IStream for encoder output.
    unsafe {
        let hr = CreateStreamOnHGlobal(0, 1, &mut stream);
        check_hr("CreateStreamOnHGlobal", hr)?;
    }

    let mut encoder: LPVOID = null_mut();
    unsafe {
        let vtbl = (*factory.as_ptr()).vtable;
        let hr = ((*vtbl).CreateEncoder)(factory.as_ptr(), container, null(), &mut encoder);
        check_hr("IWICImagingFactory::CreateEncoder", hr)?;
    }
    let encoder = ComPtr::from_raw(encoder.cast::<IWICBitmapEncoder>());

    unsafe {
        let vtbl = (*encoder.as_ptr()).vtable;
        let hr = ((*vtbl).Initialize)(encoder.as_ptr(), stream, WICBitmapEncoderNoCache);
        check_hr("IWICBitmapEncoder::Initialize", hr)?;
    }

    let mut frame: LPVOID = null_mut();
    let mut property_bag: LPVOID = null_mut();
    unsafe {
        let vtbl = (*encoder.as_ptr()).vtable;
        let hr = ((*vtbl).CreateNewFrame)(encoder.as_ptr(), &mut frame, &mut property_bag);
        check_hr("IWICBitmapEncoder::CreateNewFrame", hr)?;
    }
    let frame = ComPtr::from_raw(frame.cast::<IWICBitmapFrameEncode>());

    if codec == ImageBlobCodec::Jpeg && !property_bag.is_null() {
        set_jpeg_quality(property_bag, jpeg_quality)?;
    }
    if !property_bag.is_null() {
        unsafe { release_unknown(property_bag) };
    }

    unsafe {
        let vtbl = (*frame.as_ptr()).vtable;
        let hr = ((*vtbl).Initialize)(frame.as_ptr(), null_mut());
        check_hr("IWICBitmapFrameEncode::Initialize", hr)?;
    }

    unsafe {
        let vtbl = (*frame.as_ptr()).vtable;
        let hr = ((*vtbl).SetSize)(frame.as_ptr(), width, height);
        check_hr("IWICBitmapFrameEncode::SetSize", hr)?;
    }
    unsafe {
        let vtbl = (*frame.as_ptr()).vtable;
        let mut pixel_format = GUID_WICPixelFormat32bppBGRA;
        let hr = ((*vtbl).SetPixelFormat)(frame.as_ptr(), &mut pixel_format);
        check_hr("IWICBitmapFrameEncode::SetPixelFormat", hr)?;
    }
    let stride = width * 4;
    unsafe {
        let vtbl = (*frame.as_ptr()).vtable;
        let hr = ((*vtbl).WritePixels)(
            frame.as_ptr(),
            height,
            stride,
            pixels.len() as u32,
            pixels.as_ptr(),
        );
        check_hr("IWICBitmapFrameEncode::WritePixels", hr)?;
    }

    unsafe {
        let vtbl = (*frame.as_ptr()).vtable;
        check_hr(
            "IWICBitmapFrameEncode::Commit",
            ((*vtbl).Commit)(frame.as_ptr()),
        )?;
    }
    unsafe {
        let vtbl = (*encoder.as_ptr()).vtable;
        check_hr(
            "IWICBitmapEncoder::Commit",
            ((*vtbl).Commit)(encoder.as_ptr()),
        )?;
    }

    read_stream_bytes(stream)
}

fn set_jpeg_quality(property_bag: LPVOID, quality: u8) -> Result<()> {
    let q = (quality.clamp(1, 100) as f32) / 100.0;
    let mut name = wide("ImageQuality");
    let option = PROPBAG2 {
        dwType: 0,
        vt: VT_R4,
        cfType: 0,
        dwHint: 0,
        pstrName: name.as_mut_ptr(),
        clsid: GUID::default(),
    };
    let mut var = MaybeUninit::<VARIANT>::uninit();
    unsafe {
        VariantInit(var.as_mut_ptr());
        (*var.as_mut_ptr()).vt = VT_R4;
        (*var.as_mut_ptr()).data.fltVal = q;
        let bag = property_bag as *mut IPropertyBag2;
        let vtbl = (*bag).vtable;
        let hr = ((*vtbl).Write)(bag, 1, &option, var.as_mut_ptr());
        let _ = VariantClear(var.as_mut_ptr());
        check_hr("IPropertyBag2::Write(ImageQuality)", hr)
    }
}

fn read_stream_bytes(stream: LPVOID) -> Result<Vec<u8>> {
    let mut hglobal: ffi::HGLOBAL = 0;
    unsafe {
        check_hr(
            "GetHGlobalFromStream",
            GetHGlobalFromStream(stream, &mut hglobal),
        )?;
        release_unknown(stream);
        let size = GlobalSize(hglobal);
        if size == 0 {
            return Err(ClipError::Other("WIC encoder produced empty stream".into()));
        }
        let ptr = GlobalLock(hglobal);
        if ptr.is_null() {
            return Err(ClipError::Other("GlobalLock failed on WIC stream".into()));
        }
        let bytes = core::slice::from_raw_parts(ptr as *const u8, size).to_vec();
        let _ = GlobalUnlock(hglobal);
        Ok(bytes)
    }
}

/// Decode WIC PNG/JPEG payload bytes to top-down BGRA pixels.
pub fn decode_to_bgra(payload: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let factory = create_factory()?;

    let mut stream: LPVOID = null_mut();
    unsafe {
        let hglobal = GlobalAlloc(GHND, payload.len());
        if hglobal == 0 {
            return Err(ClipError::Other(
                "GlobalAlloc failed for decode stream".into(),
            ));
        }
        let ptr = GlobalLock(hglobal);
        if ptr.is_null() {
            return Err(ClipError::Other(
                "GlobalLock failed for decode stream".into(),
            ));
        }
        core::ptr::copy_nonoverlapping(payload.as_ptr(), ptr as *mut u8, payload.len());
        let _ = GlobalUnlock(hglobal);
        let hr = CreateStreamOnHGlobal(hglobal, 1, &mut stream);
        check_hr("CreateStreamOnHGlobal(decode)", hr)?;
    }

    let mut decoder: LPVOID = null_mut();
    unsafe {
        let vtbl = (*factory.as_ptr()).vtable;
        let hr = ((*vtbl).CreateDecoderFromStream)(
            factory.as_ptr(),
            stream,
            null(),
            WICDecodeMetadataCacheOnLoad,
            &mut decoder,
        );
        check_hr("IWICImagingFactory::CreateDecoderFromStream", hr)?;
    }
    let decoder = ComPtr::from_raw(decoder.cast::<IWICBitmapDecoder>());

    let mut frame: LPVOID = null_mut();
    unsafe {
        let vtbl = (*decoder.as_ptr()).vtable;
        let hr = ((*vtbl).GetFrame)(decoder.as_ptr(), 0, &mut frame);
        check_hr("IWICBitmapDecoder::GetFrame", hr)?;
    }
    let frame = ComPtr::from_raw(frame.cast::<IWICBitmapFrameDecode>());

    let mut converter: LPVOID = null_mut();
    unsafe {
        let vtbl = (*factory.as_ptr()).vtable;
        let hr = ((*vtbl).CreateFormatConverter)(factory.as_ptr(), &mut converter);
        check_hr("IWICImagingFactory::CreateFormatConverter", hr)?;
    }
    let converter = ComPtr::from_raw(converter.cast::<IWICFormatConverter>());

    unsafe {
        let vtbl = (*converter.as_ptr()).vtable;
        let hr = ((*vtbl).Initialize)(
            converter.as_ptr(),
            frame.as_unknown().cast(),
            &GUID_WICPixelFormat32bppBGRA,
            0,
            null(),
            0.0,
            0,
        );
        check_hr("IWICFormatConverter::Initialize", hr)?;
    }

    let mut out_w = 0u32;
    let mut out_h = 0u32;
    unsafe {
        let vtbl = (*frame.as_ptr()).vtable;
        let hr = ((*vtbl).GetSize)(frame.as_ptr(), &mut out_w, &mut out_h);
        check_hr("IWICBitmapFrameDecode::GetSize", hr)?;
    }
    if out_w != width || out_h != height {
        log::warn(&format!(
            "WIC decode size {out_w}x{out_h} differs from metadata {width}x{height}"
        ));
    }

    let stride = out_w
        .checked_mul(4)
        .ok_or_else(|| ClipError::Other("decoded image stride overflow".into()))?;
    let buf_len = (stride as u64)
        .checked_mul(out_h as u64)
        .ok_or_else(|| ClipError::Other("decoded image buffer overflow".into()))?;
    let mut pixels = vec![0u8; buf_len as usize];
    let rect = WICRect {
        X: 0,
        Y: 0,
        Width: out_w,
        Height: out_h,
    };
    unsafe {
        let vtbl = (*converter.as_ptr()).vtable;
        let hr = ((*vtbl).CopyPixels)(
            converter.as_ptr(),
            &rect,
            stride,
            pixels.len() as u32,
            pixels.as_mut_ptr(),
        );
        check_hr("IWICFormatConverter::CopyPixels", hr)?;
    }

    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win32::clipboard::{encode_bgra_dib, parse_dib_to_bgra};

    #[test]
    fn factory_smoke_create_format_converter() {
        ensure_com_initialized().expect("com");
        let factory = create_factory().expect("factory");
        let mut converter: LPVOID = null_mut();
        unsafe {
            let vtbl = (*factory.as_ptr()).vtable;
            let hr = ((*vtbl).CreateFormatConverter)(factory.as_ptr(), &mut converter);
            assert!(succeeded(hr), "CreateFormatConverter failed hr={hr:#010x}");
            assert!(!converter.is_null());
            release_unknown(converter);
        }
    }

    #[test]
    fn png_round_trip_byte_equality() {
        if !wic_codecs_available() {
            log::warn("skip png_round_trip: WIC PNG encoder unavailable on this system");
            return;
        }
        let width = 4u32;
        let height = 3u32;
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for i in 0..(width * height) {
            let b = (i * 3) as u8;
            pixels.extend_from_slice(&[b, b.wrapping_add(40), b.wrapping_add(80), 255]);
        }

        let encoded = encode_bgra(width, height, &pixels, ImageBlobCodec::Png, 90).expect("encode");
        assert!(!encoded.is_empty());

        let decoded = decode_to_bgra(&encoded, width, height).expect("decode");
        assert_eq!(decoded, pixels);

        let dib = encode_bgra_dib(width, height, &decoded);
        let round = parse_dib_to_bgra(&dib, 64 * 1024 * 1024).expect("parse dib");
        assert_eq!(round.pixels, pixels);
        assert_eq!(round.width, width);
        assert_eq!(round.height, height);
    }
}
