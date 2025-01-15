fn main() {
    println!("cargo:rustc-link-search=native=/usr/lib/x86_64-linux-gnu");

    cxx_build::bridge("src/vst3_cxx_bridge.rs")
        .file("src/vst3cxxbridge.cc")
        .file("/home/kevin/Compile/Audio/vst3sdk/public.sdk/source/vst/hosting/module.cpp")
        .file("/home/kevin/Compile/Audio/vst3sdk/public.sdk/source/vst/hosting/module_linux.cpp")
        .file("/home/kevin/Compile/Audio/vst3sdk/public.sdk/source/vst/hosting/plugprovider.cpp")
        .include("/home/kevin/Compile/Audio/vst3sdk")
        .std("c++23")
        .compile("vst3cxxbridge");

    println!("cargo:rerun-if-changed=src/vst3_cxx_bridge.rs");
    println!("cargo:rerun-if-changed=src/vst3cxxbridge.cc");
    println!("cargo:rerun-if-changed=include/vst3cxxbridge.h");
}
