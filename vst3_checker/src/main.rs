use std::ffi::CString;
use std::os::raw::c_char;

#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("vst3_checker/include/vst3pluginchecker.h");

        unsafe fn checkPlugin(vst3_plugin_path: *mut c_char);
    }
}


fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() == 2 {
        if let Some(vst_plugin_path) = args.get(1) {
            if vst_plugin_path.contains(',') {
                for plugin in vst_plugin_path.replace('\"', "").as_str().split(',').collect::<Vec<&str>>().iter() {
                    if plugin.ends_with(".vst3") {
                        let path = convert_string(&plugin.to_string());
                        unsafe { ffi::checkPlugin(path.as_ptr() as *mut c_char); }
                    }
                }
            }
            else {
                if vst_plugin_path.ends_with(".vst3") {
                    let path = convert_string(vst_plugin_path);
                    unsafe { ffi::checkPlugin(path.as_ptr() as *mut c_char); }
                }
            }
        }
    }
    else {
        println!("Something wrong with command line argument(s) given: {:?}", args);
    }
}

fn convert_string(vst_plugin_path: &String) -> CString {
    CString::new(vst_plugin_path.as_bytes()).unwrap_or_else(|nul_error| {
        let nul_position = nul_error.nul_position();
        let mut bytes = nul_error.into_vec();
        bytes.truncate(nul_position);
        CString::new(bytes).unwrap()
    })
}
