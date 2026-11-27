// Integration test: A2L representation of one and two dimensional measurement instances in the xcp_registry A2L writer
//
// xcplib (src/a2l.c, A2lCreateMeasurementArray_) writes measurement arrays as CHARACTERISTIC VAL_BLK with MATRIX_DIM and an IF_DATA DAQ_EVENT.
// This test documents what the xcp_registry writer used by xcpclient (--create-a2l, --fix-a2l) writes for the same objects,
// and that the A2L file loads again with the dimensions and the object type kept.

use xcp_registry::{McAddress, McDimType, McEvent, McInstance, McObjectType, McSupportData, McValueType, Registry};

const EVENT_BATCH_DONE: u16 = 0;
const EVENT_MAINLOOP: u16 = 1;

// Registry with the measurement arrays of the test
fn create_registry() -> Registry {
    let mut reg = Registry::new();
    reg.event_list.add_event(McEvent::new("batch_done", 0, EVENT_BATCH_DONE, 0)).unwrap();
    reg.event_list.add_event(McEvent::new("mainloop", 0, EVENT_MAINLOOP, 0)).unwrap();

    let measurement = |comment: &'static str| McSupportData::new(McObjectType::Measurement).set_comment(comment);

    // The multi_thread_demo image: static uint8_t pixels[1536][2048], absolute addressing, event batch_done
    reg.instance_list.push(McInstance::new(
        "pixels",
        McDimType::new(McValueType::Ubyte, 2048, 1536),
        measurement("Mandelbrot iteration count per pixel"),
        McAddress::new_event_abs(EVENT_BATCH_DONE, 0x28068),
    ));

    // One dimensional array, absolute addressing
    reg.instance_list.push(McInstance::new(
        "curve",
        McDimType::new(McValueType::Float64Ieee, 16, 1),
        measurement("One dimensional array"),
        McAddress::new_event_abs(EVENT_MAINLOOP, 0x1000),
    ));

    // Two dimensional array on the stack, relative addressing (address extension 2), the event is fixed
    reg.instance_list.push(McInstance::new(
        "stack_matrix",
        McDimType::new(McValueType::Uword, 4, 3),
        measurement("Two dimensional array on the stack"),
        McAddress::new_event_dyn(0, EVENT_MAINLOOP, -64),
    ));

    reg
}

// The line of the A2L text which defines the object 'name'
fn object_line<'a>(a2l: &'a str, name: &str) -> &'a str {
    a2l.lines()
        .find(|l| l.starts_with("/begin ") && l.split_whitespace().nth(2) == Some(name))
        .unwrap_or_else(|| panic!("object '{name}' not found in the A2L file"))
}

#[test]
fn test_registry_writes_measurement_arrays_as_measurement_with_matrix_dim() {
    let reg = create_registry();
    let dir = std::env::temp_dir().join(format!("xcpclient_registry_a2l_matrix_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("matrix.a2l");
    reg.write_a2l(&path, "registry_a2l_matrix test", "matrix", "", "matrix", "XCPLITE__ACSDD", true)
        .expect("write_a2l failed");
    let a2l = std::fs::read_to_string(&path).unwrap();
    for name in ["pixels", "curve", "stack_matrix"] {
        println!("{}", object_line(&a2l, name));
    }

    // Measurement arrays are MEASUREMENTs with MATRIX_DIM, not CHARACTERISTICs
    let pixels = object_line(&a2l, "pixels");
    assert!(pixels.starts_with("/begin MEASUREMENT pixels "), "{pixels}");
    assert!(pixels.contains(" UBYTE "), "{pixels}");
    assert!(pixels.contains(" MATRIX_DIM 2048 1536"), "{pixels}");
    // Absolute addressing: the event is a default event, the tool may choose another one
    assert!(pixels.contains("/begin DEFAULT_EVENT_LIST EVENT 0 /end DEFAULT_EVENT_LIST"), "{pixels}");

    let curve = object_line(&a2l, "curve");
    assert!(curve.starts_with("/begin MEASUREMENT curve "), "{curve}");
    assert!(curve.contains(" MATRIX_DIM 16 ") || curve.ends_with(" MATRIX_DIM 16"), "{curve}");
    assert!(!curve.contains("MATRIX_DIM 16 1"), "{curve}");

    // Relative addressing: the event is fixed
    let stack_matrix = object_line(&a2l, "stack_matrix");
    assert!(stack_matrix.starts_with("/begin MEASUREMENT stack_matrix "), "{stack_matrix}");
    assert!(stack_matrix.contains(" MATRIX_DIM 4 3"), "{stack_matrix}");
    assert!(stack_matrix.contains("/begin DAQ_EVENT FIXED_EVENT_LIST EVENT 1 /end DAQ_EVENT"), "{stack_matrix}");

    // The A2L file loads again, dimensions and object type are kept
    let mut reg2 = Registry::new();
    reg2.load_a2l(&path, true, true, true, true).expect("load_a2l failed");
    for (name, x_dim, y_dim) in [("pixels", 2048, 1536), ("curve", 16, 1), ("stack_matrix", 4, 3)] {
        let instance = reg2
            .instance_list
            .get_instance(name, McObjectType::Measurement, None)
            .unwrap_or_else(|| panic!("measurement '{name}' not found after reload"));
        assert_eq!((instance.x_dim(), instance.y_dim()), (x_dim, y_dim), "{name}");
        assert_eq!(instance.size(), x_dim as usize * y_dim as usize * instance.value_size(), "{name}");
    }

    std::fs::remove_dir_all(&dir).ok();
}
