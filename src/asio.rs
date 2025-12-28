use crate::{visualizer, AudioRing};
use asio_sys::{
    asio_import::{
        ASIOBufferInfo, ASIOCallbacks, ASIOChannelInfo, ASIOCreateBuffers, ASIODriverInfo,
        ASIOGetBufferSize, ASIOGetChannelInfo, ASIOGetChannels, ASIOInit, ASIOSampleRate,
        ASIOStart, ASIOTime, AsioDrivers,
    },
    errors::AsioErrorWrapper,
};
use std::{ptr, sync::Arc};

static mut RING: Option<Arc<AudioRing>> = None;
static mut BUFFER_SIZE: usize = 0;
static mut CHANNELS: usize = 0;
static mut ASIO_BUFFERS: *mut ASIOBufferInfo = std::ptr::null_mut();

// Global visualizer reference (unsafe)
static mut VISUALIZER: Option<Arc<visualizer::AudioVisualizer>> = None;

pub fn set_visualizer(visualizer: Arc<visualizer::AudioVisualizer>) {
    unsafe {
        VISUALIZER = Some(visualizer);
    }
}
/*
unsafe extern "C" fn buffer_switch(double_buffer_index: i32, _direct: i32) {
    let ring = RING.as_ref().unwrap();

    let frames = BUFFER_SIZE;
    let chans = CHANNELS;

    let mut out = vec![0.0f32; frames * chans];

    for ch in 0..chans {
        let buf_info = &*ASIO_BUFFERS.add(ch); // first 'ins' entries are inputs
        let buffer_ptr = buf_info.buffers[double_buffer_index as usize] as *const f32;
        if !buffer_ptr.is_null() {
            let src = std::slice::from_raw_parts(buffer_ptr, frames);
            for i in 0..frames {
                out[i * chans + ch] = src[i];
            }
        }
    }

    let amplitude = calculate_rms(&out);
    ring.push(out);

    if let Some(ref visualizer) = VISUALIZER {
        visualizer.update_amplitude(amplitude);
    }
} */

use std::slice;

enum AsioSampleType {
    ASIOSTInt16MSB = 0,
    ASIOSTInt24MSB = 1, // used for 20 bits as well
    ASIOSTInt32MSB = 2,
    ASIOSTFloat32MSB = 3, // IEEE 754 32 bit float
    ASIOSTFloat64MSB = 4, // IEEE 754 64 bit double float

    // these are used for 32 bit data buffer, with different alignment of the data inside
    // 32 bit PCI bus systems can be more easily used with these
    ASIOSTInt32MSB16 = 8,  // 32 bit data with 16 bit alignment
    ASIOSTInt32MSB18 = 9,  // 32 bit data with 18 bit alignment
    ASIOSTInt32MSB20 = 10, // 32 bit data with 20 bit alignment
    ASIOSTInt32MSB24 = 11, // 32 bit data with 24 bit alignment

    ASIOSTInt16LSB = 16,
    ASIOSTInt24LSB = 17, // used for 20 bits as well
    ASIOSTInt32LSB = 18,
    ASIOSTFloat32LSB = 19, // IEEE 754 32 bit float, as found on Intel x86 architecture
    ASIOSTFloat64LSB = 20, // IEEE 754 64 bit double float, as found on Intel x86 architecture

    // these are used for 32 bit data buffer, with different alignment of the data inside
    // 32 bit PCI bus systems can more easily used with these
    ASIOSTInt32LSB16 = 24, // 32 bit data with 18 bit alignment
    ASIOSTInt32LSB18 = 25, // 32 bit data with 18 bit alignment
    ASIOSTInt32LSB20 = 26, // 32 bit data with 20 bit alignment
    ASIOSTInt32LSB24 = 27, // 32 bit data with 24 bit alignment

    //	ASIO DSD format.
    ASIOSTDSDInt8LSB1 = 32, // DSD 1 bit data, 8 samples per byte. First sample in Least significant bit.
    ASIOSTDSDInt8MSB1 = 33, // DSD 1 bit data, 8 samples per byte. First sample in Most significant bit.
    ASIOSTDSDInt8NER8 = 40, // DSD 8 bit data, 1 sample per byte. No Endianness required.

    ASIOSTLastEntry,
}

impl From<i32> for AsioSampleType {
    fn from(value: i32) -> Self {
        unsafe { std::mem::transmute(value as i8) }
    }
}

unsafe extern "C" fn buffer_switch(double_buffer_index: i32, _direct: i32) {
    let ring = RING.as_ref().unwrap();
    let frames = BUFFER_SIZE;
    let chans = CHANNELS;
    let mut out = vec![0.0f32; frames * chans];

    for ch in 0..chans {
        let buf_info = &*ASIO_BUFFERS.add(ch);

        // Get pointer to the correct double buffer
        let buffer_ptr = buf_info.buffers[double_buffer_index as usize];
        if buffer_ptr.is_null() {
            continue;
        }

        // Query the sample type for this channel
        let mut info: ASIOChannelInfo = std::mem::zeroed();
        info.channel = buf_info.channelNum;
        info.isInput = buf_info.isInput;
        ASIOGetChannelInfo(&mut info);

        match AsioSampleType::from(info.type_) {
            AsioSampleType::ASIOSTFloat32LSB => {
                let slice = slice::from_raw_parts(buffer_ptr as *const f32, frames);
                for i in 0..frames {
                    out[i * chans + ch] = slice[i];
                }
            }
            AsioSampleType::ASIOSTInt16LSB => {
                let slice = slice::from_raw_parts(buffer_ptr as *const i16, frames);
                for i in 0..frames {
                    out[i * chans + ch] = slice[i] as f32 / 32768.0;
                }
            }
            AsioSampleType::ASIOSTInt32LSB => {
                let slice = slice::from_raw_parts(buffer_ptr as *const i32, frames);
                for i in 0..frames {
                    out[i * chans + ch] = slice[i] as f32 / 2147483648.0;
                }
            }
            AsioSampleType::ASIOSTFloat64LSB => {
                let slice = slice::from_raw_parts(buffer_ptr as *const f64, frames);
                for i in 0..frames {
                    out[i * chans + ch] = slice[i] as f32;
                }
            }
            _ => panic!("Unsupported ASIO sample type for channel {}", ch),
        }
    }

    // Push interleaved f32 buffer to ring
    let amplitude = calculate_rms(&out);
    ring.push(out);

    if let Some(ref visualizer) = VISUALIZER {
        visualizer.update_amplitude(amplitude);
    }
}

fn calculate_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    let sum: f64 = samples.iter().map(|&x| x as f64 * x as f64).sum();
    let mean = sum / samples.len() as f64;
    mean.sqrt() as f32
}

unsafe extern "C" fn sample_rate_changed(rate: ASIOSampleRate) {}
unsafe extern "C" fn asio_message(
    selector: i32,
    value: i32,
    msg: *mut std::ffi::c_void,
    opt: *mut f64,
) -> i32 {
    0
}

unsafe extern "C" fn buffer_switch_time_info(
    params: *mut ASIOTime,
    index: i32,
    direct: i32,
) -> *mut ASIOTime {
    buffer_switch(index, direct);
    ptr::null_mut()
}

static mut CALLBACKS: ASIOCallbacks = ASIOCallbacks {
    bufferSwitch: Some(buffer_switch),
    bufferSwitchTimeInfo: Some(buffer_switch_time_info),
    sampleRateDidChange: Some(sample_rate_changed),
    asioMessage: Some(asio_message),
};

pub unsafe fn start_asio(ring: Arc<AudioRing>) -> anyhow::Result<()> {
    RING = Some(ring);

    // Create AsioDrivers instance to enumerate drivers
    let mut drivers = AsioDrivers::new();

    const MAX_DRIVERS: usize = 32;
    const MAX_NAME_LEN: usize = 32;
    let mut name_storage = vec![[0i8; MAX_NAME_LEN]; MAX_DRIVERS];
    let mut name_ptrs: Vec<*mut i8> = name_storage
        .iter_mut()
        .map(|buf| buf.as_mut_ptr())
        .collect();

    // If NUX driver not found, enumerate all drivers and find one that contains 'NUX' in its name

    let num_drivers = drivers.getDriverNames(name_ptrs.as_mut_ptr(), MAX_DRIVERS as i32);

    // Print the driver names
    for i in 0..num_drivers {
        let driver_ptr = name_ptrs[i as usize];

        if !driver_ptr.is_null() {
            let driver_cstr = std::ffi::CStr::from_ptr(driver_ptr);
            if let Ok(driver_str) = driver_cstr.to_str() {
                // Check if this is the NUX driver (case-insensitive search)
                if driver_str.contains("NUX") && drivers.loadDriver(driver_ptr) {
                    println!("Driver {}: {} Loaded", i, driver_str);
                    break;
                }
            }
        }
    }

    // 2. init
    let mut info: ASIODriverInfo = unsafe { std::mem::zeroed() };
    let rc = ASIOInit(&mut info);
    assert_eq!(rc, AsioErrorWrapper::ASE_OK as i32);

    // 3. channels
    let mut ins = 0;
    let mut outs = 0;
    let rc = ASIOGetChannels(&mut ins, &mut outs);
    assert_eq!(rc, AsioErrorWrapper::ASE_OK as i32);
    CHANNELS = ins as usize;

    // 4. buffer size
    let mut min = 0;
    let mut max = 0;
    let mut pref = 0;
    let mut gran = 0;
    let rc = ASIOGetBufferSize(&mut min, &mut max, &mut pref, &mut gran);
    assert_eq!(rc, AsioErrorWrapper::ASE_OK as i32);

    BUFFER_SIZE = pref as usize;

    println!(
        "ASIO driver info: {:?}",
        std::ffi::CStr::from_ptr(&info.errorMessage as *const i8)
    );
    println!("Channels: ins={}, outs={}", ins, outs);
    println!(
        "Buffer size: min={}, max={}, pref={}, gran={}",
        min, max, pref, gran
    );

    // Prepare input buffers
    let mut buffers = Vec::new();
    for i in 0..ins {
        buffers.push(ASIOBufferInfo {
            isInput: 1,    // 1 = input
            channelNum: i, // unique index
            buffers: [ptr::null_mut(), ptr::null_mut()],
        });
    }

    // Prepare output buffers
    for i in 0..outs {
        buffers.push(ASIOBufferInfo {
            isInput: 0,    // 0 = output
            channelNum: i, // unique index
            buffers: [ptr::null_mut(), ptr::null_mut()],
        });
    }

    ASIO_BUFFERS = buffers.leak().as_mut_ptr();

    let rc = unsafe {
        ASIOCreateBuffers(
            ASIO_BUFFERS,
            ins + outs, // total number of buffers, inputs + outputs
            BUFFER_SIZE as i32,
            &raw mut CALLBACKS,
        )
    };

    assert_eq!(rc, AsioErrorWrapper::ASE_OK as i32);
    let rc = ASIOStart();
    assert_eq!(rc, AsioErrorWrapper::ASE_OK as i32);

    Ok(())
}
