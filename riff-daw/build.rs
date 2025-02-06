fn main() {
    println!("cargo:rustc-link-search=native=/usr/lib/x86_64-linux-gnu");

    cxx_build::bridge("src/vst3_cxx_bridge.rs")
        .file("src/vst3cxxbridge.cc")
        .file("../lib/vst3sdk/public.sdk/source/vst/hosting/module.cpp")
        .file("../lib/vst3sdk/public.sdk/source/vst/hosting/module_linux.cpp")
        .file("../lib/vst3sdk/public.sdk/source/vst/hosting/plugprovider.cpp")
        .include("../lib/vst3sdk")
        .std("c++23")
        .compile("vst3cxxbridge");

    println!("cargo:rerun-if-changed=src/vst3_cxx_bridge.rs");
    println!("cargo:rerun-if-changed=src/vst3cxxbridge.cc");
    println!("cargo:rerun-if-changed=include/vst3cxxbridge.h");
}
