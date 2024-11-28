fn main() {
    tonic_build::compile_protos("proto/steganography.proto")
        .unwrap_or_else(|e| panic!("Failed to compile protos {:?}", e));
    tonic_build::compile_protos("proto/port_grabber.proto")
        .unwrap_or_else(|e| panic!("Failed to compile protos {:?}", e));
}
