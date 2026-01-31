mod common;

use aelys_bytecode::{AelysArray, AelysVec, ArrayData, TypeTag, Value};
use common::{assert_aelys_bool, assert_aelys_error_contains, assert_aelys_int, run_aelys, run_aelys_ok};

#[test]
fn test_array_new_ints() {
    let arr = AelysArray::new_ints(5);
    assert_eq!(arr.len(), 5);
    assert!(!arr.is_empty());
    assert_eq!(arr.type_tag(), TypeTag::Int);

    // Zero-initialized
    for i in 0..5 {
        assert_eq!(arr.get(i), Some(Value::int(0)));
    }
}

#[test]
fn test_array_new_floats() {
    let arr = AelysArray::new_floats(3);
    assert_eq!(arr.len(), 3);
    assert_eq!(arr.type_tag(), TypeTag::Float);

    for i in 0..3 {
        assert_eq!(arr.get(i), Some(Value::float(0.0)));
    }
}

#[test]
fn test_array_new_bools() {
    let arr = AelysArray::new_bools(4);
    assert_eq!(arr.len(), 4);
    assert_eq!(arr.type_tag(), TypeTag::Bool);

    for i in 0..4 {
        assert_eq!(arr.get(i), Some(Value::bool(false)));
    }
}

#[test]
fn test_array_new_objects() {
    let arr = AelysArray::new_objects(2);
    assert_eq!(arr.len(), 2);
    assert_eq!(arr.type_tag(), TypeTag::Object);

    for i in 0..2 {
        assert!(arr.get(i).unwrap().is_null());
    }
}

#[test]
fn test_array_from_ints() {
    let arr = AelysArray::from_ints(vec![1, 2, 3, 4, 5]);
    assert_eq!(arr.len(), 5);
    assert_eq!(arr.type_tag(), TypeTag::Int);

    assert_eq!(arr.get(0), Some(Value::int(1)));
    assert_eq!(arr.get(2), Some(Value::int(3)));
    assert_eq!(arr.get(4), Some(Value::int(5)));
    assert_eq!(arr.get(5), None); // Out of bounds
}

#[test]
fn test_array_from_floats() {
    let arr = AelysArray::from_floats(vec![1.5, 2.5, 3.5]);
    assert_eq!(arr.len(), 3);
    assert_eq!(arr.type_tag(), TypeTag::Float);

    assert_eq!(arr.get(0), Some(Value::float(1.5)));
    assert_eq!(arr.get(1), Some(Value::float(2.5)));
    assert_eq!(arr.get(2), Some(Value::float(3.5)));
}

#[test]
fn test_array_from_bools() {
    let arr = AelysArray::from_bools(vec![true, false, true]);
    assert_eq!(arr.len(), 3);
    assert_eq!(arr.type_tag(), TypeTag::Bool);

    assert_eq!(arr.get(0), Some(Value::bool(true)));
    assert_eq!(arr.get(1), Some(Value::bool(false)));
    assert_eq!(arr.get(2), Some(Value::bool(true)));
}

#[test]
fn test_array_set() {
    let mut arr = AelysArray::new_ints(3);

    assert!(arr.set(0, Value::int(10)));
    assert!(arr.set(1, Value::int(20)));
    assert!(arr.set(2, Value::int(30)));

    assert_eq!(arr.get(0), Some(Value::int(10)));
    assert_eq!(arr.get(1), Some(Value::int(20)));
    assert_eq!(arr.get(2), Some(Value::int(30)));

    // Out of bounds set returns false
    assert!(!arr.set(3, Value::int(40)));

    // Wrong type set returns false (ints array, trying to set float)
    assert!(!arr.set(0, Value::float(1.5)));
}

#[test]
fn test_array_empty() {
    let arr = AelysArray::new_ints(0);
    assert_eq!(arr.len(), 0);
    assert!(arr.is_empty());
    assert_eq!(arr.get(0), None);
}

#[test]
fn test_array_size_bytes() {
    let arr_ints = AelysArray::from_ints(vec![1, 2, 3, 4]);
    let arr_bools = AelysArray::from_bools(vec![true, false, true, false]);

    // Ints use 8 bytes/element, bools use 1 byte/element
    // Size includes struct overhead
    assert!(arr_ints.size_bytes() > arr_bools.size_bytes());
}

#[test]
fn test_vec_new_empty() {
    let v = AelysVec::new_ints();
    assert_eq!(v.len(), 0);
    assert!(v.is_empty());
    assert_eq!(v.type_tag(), TypeTag::Int);
}

#[test]
fn test_vec_push_pop() {
    let mut v = AelysVec::new_ints();

    assert!(v.push(Value::int(1)));
    assert!(v.push(Value::int(2)));
    assert!(v.push(Value::int(3)));

    assert_eq!(v.len(), 3);
    assert!(!v.is_empty());

    assert_eq!(v.pop(), Some(Value::int(3)));
    assert_eq!(v.pop(), Some(Value::int(2)));
    assert_eq!(v.pop(), Some(Value::int(1)));
    assert_eq!(v.pop(), None);

    assert!(v.is_empty());
}

#[test]
fn test_vec_push_wrong_type() {
    let mut v = AelysVec::new_ints();

    // Push wrong type returns false
    assert!(!v.push(Value::float(1.5)));
    assert!(!v.push(Value::bool(true)));

    // Vec remains unchanged
    assert!(v.is_empty());
}

#[test]
fn test_vec_get_set() {
    let mut v = AelysVec::from_ints(vec![10, 20, 30]);

    assert_eq!(v.get(0), Some(Value::int(10)));
    assert_eq!(v.get(1), Some(Value::int(20)));
    assert_eq!(v.get(2), Some(Value::int(30)));
    assert_eq!(v.get(3), None);

    assert!(v.set(1, Value::int(25)));
    assert_eq!(v.get(1), Some(Value::int(25)));

    // Out of bounds set
    assert!(!v.set(5, Value::int(50)));
}

#[test]
fn test_vec_reserve() {
    let mut v = AelysVec::new_floats();
    assert_eq!(v.capacity(), 0);

    v.reserve(10);
    assert!(v.capacity() >= 10);
    assert!(v.is_empty()); // Reserve doesn't add elements
}

#[test]
fn test_vec_clear() {
    let mut v = AelysVec::from_bools(vec![true, false, true]);
    assert_eq!(v.len(), 3);

    v.clear();
    assert!(v.is_empty());
    assert_eq!(v.len(), 0);
}

#[test]
fn test_vec_shrink_to_fit() {
    let mut v = AelysVec::new_ints();
    v.reserve(100);
    assert!(v.capacity() >= 100);

    v.push(Value::int(1));
    v.shrink_to_fit();
    // Capacity should be reduced (implementation-dependent exact value)
    assert!(v.capacity() >= 1);
}

#[test]
fn test_vec_to_array() {
    let v = AelysVec::from_ints(vec![1, 2, 3]);
    let arr = v.to_array();

    assert_eq!(arr.len(), 3);
    assert_eq!(arr.type_tag(), TypeTag::Int);
    assert_eq!(arr.get(0), Some(Value::int(1)));
    assert_eq!(arr.get(1), Some(Value::int(2)));
    assert_eq!(arr.get(2), Some(Value::int(3)));
}

#[test]
fn test_vec_with_capacity() {
    let v = AelysVec::with_capacity_ints(10);
    assert!(v.is_empty());
    assert!(v.capacity() >= 10);
}

#[test]
fn test_type_tag_from_u8() {
    assert_eq!(TypeTag::from_u8(0), Some(TypeTag::Int));
    assert_eq!(TypeTag::from_u8(1), Some(TypeTag::Float));
    assert_eq!(TypeTag::from_u8(2), Some(TypeTag::Bool));
    assert_eq!(TypeTag::from_u8(3), Some(TypeTag::Object));
    assert_eq!(TypeTag::from_u8(4), None);
    assert_eq!(TypeTag::from_u8(255), None);
}

#[test]
fn test_array_data_accessors() {
    let data_ints = ArrayData::Ints(vec![1, 2, 3].into_boxed_slice());
    assert!(data_ints.as_ints().is_some());
    assert!(data_ints.as_floats().is_none());
    assert!(data_ints.as_bools().is_none());
    assert!(data_ints.as_objects().is_none());

    let data_floats = ArrayData::Floats(vec![1.0, 2.0].into_boxed_slice());
    assert!(data_floats.as_floats().is_some());
    assert!(data_floats.as_ints().is_none());

    let data_bools = ArrayData::Bools(vec![1, 0, 1].into_boxed_slice());
    assert!(data_bools.as_bools().is_some());
    assert!(data_bools.as_floats().is_none());

    let data_objects = ArrayData::Objects(vec![Value::null()].into_boxed_slice());
    assert!(data_objects.as_objects().is_some());
    assert!(data_objects.as_bools().is_none());
}

#[test]
fn test_array_data_mutable_accessors() {
    let mut data = ArrayData::Ints(vec![1, 2, 3].into_boxed_slice());

    if let Some(ints) = data.as_ints_mut() {
        ints[0] = 100;
    }

    assert_eq!(data.as_ints().unwrap()[0], 100);
}

#[test]
fn test_e2e_empty_array_literal() {
    let result = run_aelys_ok("let arr = []; 0");
    assert_eq!(result.as_int(), Some(0));
}

#[test]
fn test_e2e_int_array_literal() {
    // Simple array with ints
    assert_aelys_int("let arr = [1, 2, 3]; arr[0]", 1);
    assert_aelys_int("let arr = [1, 2, 3]; arr[1]", 2);
    assert_aelys_int("let arr = [1, 2, 3]; arr[2]", 3);
}

#[test]
fn test_e2e_array_length() {
    assert_aelys_int("let arr = [1, 2, 3, 4, 5]; arr.len()", 5);
    assert_aelys_int("let arr = []; arr.len()", 0);
}

#[test]
fn test_e2e_array_index_expression() {
    // Index with computed expression
    assert_aelys_int("let arr = [10, 20, 30]; let i = 1; arr[i]", 20);
    assert_aelys_int("let arr = [10, 20, 30]; arr[1 + 1]", 30);
}

#[test]
fn test_e2e_array_store() {
    assert_aelys_int("let arr = [1, 2, 3]; arr[0] = 100; arr[0]", 100);
    assert_aelys_int("let arr = [1, 2, 3]; arr[1] = 200; arr[1]", 200);
}

#[test]
fn test_e2e_float_array() {
    let result = run_aelys("let arr = [1.5, 2.5, 3.5]; arr[1]");
    assert_eq!(result.as_float(), Some(2.5));
}

#[test]
fn test_e2e_bool_array() {
    assert_aelys_bool("let arr = [true, false, true]; arr[0]", true);
    assert_aelys_bool("let arr = [true, false, true]; arr[1]", false);
}

#[test]
fn test_e2e_nested_array_access() {
    // Array in function
    assert_aelys_int(
        r#"
        fn get_second(arr) {
            arr[1]
        }
        let a = [10, 20, 30];
        get_second(a)
        "#,
        20,
    );
}

#[test]
fn test_e2e_array_in_loop() {
    assert_aelys_int(
        r#"
        let arr = [1, 2, 3, 4, 5];
        let mut sum = 0;
        let mut i = 0;
        while i < arr.len() {
            sum = sum + arr[i];
            i = i + 1;
        }
        sum
        "#,
        15, // 1+2+3+4+5
    );
}

#[test]
fn test_e2e_array_trailing_comma() {
    // Trailing comma is allowed
    assert_aelys_int("let arr = [1, 2, 3,]; arr[2]", 3);
}

#[test]
fn test_e2e_empty_vec_literal() {
    let result = run_aelys_ok("let v = Vec[]; 0");
    assert_eq!(result.as_int(), Some(0));
}

#[test]
fn test_e2e_vec_literal() {
    assert_aelys_int("let v = Vec[1, 2, 3]; v[0]", 1);
    assert_aelys_int("let v = Vec[1, 2, 3]; v[2]", 3);
}

#[test]
fn test_e2e_vec_length() {
    assert_aelys_int("let v = Vec[1, 2, 3, 4]; v.len()", 4);
}

#[test]
fn test_e2e_vec_push() {
    assert_aelys_int(
        r#"
        let v = Vec[1, 2];
        v.push(3);
        v[2]
        "#,
        3,
    );
}

#[test]
fn test_e2e_vec_pop() {
    assert_aelys_int(
        r#"
        let v = Vec[1, 2, 3];
        v.pop()
        "#,
        3,
    );
}

#[test]
fn test_e2e_vec_push_pop_sequence() {
    assert_aelys_int(
        r#"
        let v = Vec[];
        v.push(10);
        v.push(20);
        v.push(30);
        let a = v.pop();
        let b = v.pop();
        a + b
        "#,
        50, // 30 + 20
    );
}

#[test]
fn test_e2e_vec_store() {
    assert_aelys_int("let v = Vec[1, 2, 3]; v[1] = 100; v[1]", 100);
}

#[test]
fn test_e2e_typed_array_int() {
    assert_aelys_int("let arr = Array<Int>[1, 2, 3]; arr[1]", 2);
}

#[test]
fn test_e2e_typed_array_float() {
    let result = run_aelys("let arr = Array<Float>[1.0, 2.0, 3.0]; arr[2]");
    assert_eq!(result.as_float(), Some(3.0));
}

#[test]
fn test_e2e_typed_array_bool() {
    assert_aelys_bool("let arr = Array<Bool>[true, false]; arr[1]", false);
}

#[test]
fn test_e2e_typed_vec_int() {
    assert_aelys_int("let v = Vec<Int>[5, 10, 15]; v[0]", 5);
}

#[test]
fn test_e2e_array_oob_read() {
    assert_aelys_error_contains("let arr = [1, 2, 3]; arr[10]", "out of bounds");
}

#[test]
fn test_e2e_array_oob_write() {
    assert_aelys_error_contains("let arr = [1, 2, 3]; arr[5] = 10", "out of bounds");
}

#[test]
fn test_e2e_array_negative_index() {
    assert_aelys_error_contains("let arr = [1, 2, 3]; arr[-1]", "");
}

#[test]
fn test_e2e_vec_oob_read() {
    assert_aelys_error_contains("let v = Vec[1, 2]; v[5]", "out of bounds");
}

#[test]
fn test_e2e_array_sum_loop() {
    assert_aelys_int(
        r#"
        let arr = [10, 20, 30, 40, 50];
        let mut sum = 0;
        let mut i = 0;
        while i < arr.len() {
            sum = sum + arr[i];
            i = i + 1;
        }
        sum
        "#,
        150,
    );
}

#[test]
fn test_e2e_array_modify_in_loop() {
    assert_aelys_int(
        r#"
        let arr = [1, 2, 3, 4, 5];
        let mut i = 0;
        while i < arr.len() {
            arr[i] = arr[i] * 2;
            i = i + 1;
        }
        arr[0] + arr[1] + arr[2] + arr[3] + arr[4]
        "#,
        30, // 2+4+6+8+10
    );
}

#[test]
fn test_e2e_array_find_max() {
    assert_aelys_int(
        r#"
        let arr = [5, 2, 9, 1, 7];
        let mut max = arr[0];
        let mut i = 1;
        while i < arr.len() {
            if arr[i] > max {
                max = arr[i];
            }
            i = i + 1;
        }
        max
        "#,
        9,
    );
}

#[test]
fn test_e2e_array_swap() {
    assert_aelys_int(
        r#"
        let arr = [10, 20];
        let tmp = arr[0];
        arr[0] = arr[1];
        arr[1] = tmp;
        arr[0] * 10 + arr[1]
        "#,
        210, // 20*10 + 10
    );
}

#[test]
fn test_e2e_array_passed_to_function() {
    assert_aelys_int(
        r#"
        fn sum_arr(a) -> int {
            let mut s = 0;
            let mut i = 0;
            while i < a.len() {
                s = s + a[i];
                i = i + 1;
            }
            return s
        }
        let arr = [1, 2, 3, 4];
        sum_arr(arr)
        "#,
        10,
    );
}

#[test]
fn test_e2e_array_returned_from_function() {
    assert_aelys_int(
        r#"
        fn make_arr() {
            return [100, 200, 300]
        }
        let a = make_arr();
        a[1]
        "#,
        200,
    );
}

#[test]
fn test_e2e_array_nested_access() {
    assert_aelys_int(
        r#"
        let a = [1, 2, 3];
        let b = [10, 20, 30];
        a[0] + b[a[1]]
        "#,
        31, // 1 + b[2] = 1 + 30
    );
}

#[test]
fn test_e2e_vec_build_and_sum() {
    assert_aelys_int(
        r#"
        let v = Vec[];
        v.push(1);
        v.push(2);
        v.push(3);
        v.push(4);
        v.push(5);
        let mut sum = 0;
        let mut i = 0;
        while i < v.len() {
            sum = sum + v[i];
            i = i + 1;
        }
        sum
        "#,
        15,
    );
}

#[test]
fn test_e2e_vec_pop_all() {
    assert_aelys_int(
        r#"
        let v = Vec[10, 20, 30];
        let a = v.pop();
        let b = v.pop();
        let c = v.pop();
        a + b + c
        "#,
        60,
    );
}

#[test]
fn test_e2e_vec_modify_elements() {
    assert_aelys_int(
        r#"
        let v = Vec[1, 2, 3];
        v[0] = 100;
        v[1] = 200;
        v[2] = 300;
        v[0] + v[1] + v[2]
        "#,
        600,
    );
}

#[test]
fn test_e2e_vec_capacity_after_reserve() {
    assert_aelys_bool(
        r#"
        let v = Vec[];
        v.reserve(50);
        v.capacity() >= 50
        "#,
        true,
    );
}

#[test]
fn test_e2e_vec_grow_dynamically() {
    assert_aelys_int(
        r#"
        let v = Vec[];
        let mut i = 0;
        while i < 100 {
            v.push(i);
            i = i + 1;
        }
        v.len()
        "#,
        100,
    );
}

#[test]
fn test_e2e_vec_passed_to_function() {
    assert_aelys_int(
        r#"
        fn double_all(v) {
            let mut i = 0;
            while i < v.len() {
                v[i] = v[i] * 2;
                i = i + 1;
            }
        }
        let v = Vec[1, 2, 3];
        double_all(v);
        v[0] + v[1] + v[2]
        "#,
        12, // 2+4+6
    );
}

#[test]
fn test_e2e_vec_stack_operations() {
    assert_aelys_int(
        r#"
        let stack = Vec[];
        stack.push(1);
        stack.push(2);
        stack.push(3);
        let a = stack.pop();
        stack.push(4);
        let b = stack.pop();
        let c = stack.pop();
        a * 100 + b * 10 + c
        "#,
        342, // 3*100 + 4*10 + 2
    );
}

#[test]
fn test_e2e_float_array_sum() {
    let result = run_aelys(
        r#"
        let arr = [1.5, 2.5, 3.0];
        let mut sum = 0.0;
        let mut i = 0;
        while i < arr.len() {
            sum = sum + arr[i];
            i = i + 1;
        }
        sum
        "#,
    );
    assert_eq!(result.as_float(), Some(7.0));
}

#[test]
fn test_e2e_float_vec_push_pop() {
    let result = run_aelys(
        r#"
        let v = Vec[1.0, 2.0];
        v.push(3.5);
        v.pop()
        "#,
    );
    assert_eq!(result.as_float(), Some(3.5));
}

#[test]
fn test_e2e_bool_array_all_true() {
    assert_aelys_bool(
        r#"
        let arr = [true, true, true];
        let mut all = true;
        let mut i = 0;
        while i < arr.len() {
            if arr[i] == false { all = false }
            i = i + 1;
        }
        all
        "#,
        true,
    );
}

#[test]
fn test_e2e_bool_array_any_true() {
    assert_aelys_bool(
        r#"
        let arr = [false, true, false];
        let mut any = false;
        let mut i = 0;
        while i < arr.len() {
            if arr[i] { any = true }
            i = i + 1;
        }
        any
        "#,
        true,
    );
}

#[test]
fn test_e2e_bool_vec_push_pop() {
    assert_aelys_bool(
        r#"
        let v = Vec[false, false];
        v.push(true);
        v.pop()
        "#,
        true,
    );
}

#[test]
fn test_e2e_single_element_array() {
    assert_aelys_int("let arr = [42]; arr[0]", 42);
    assert_aelys_int("let arr = [42]; arr.len()", 1);
}

#[test]
fn test_e2e_single_element_vec() {
    assert_aelys_int("let v = Vec[99]; v[0]", 99);
    assert_aelys_int("let v = Vec[99]; v.len()", 1);
}

#[test]
fn test_e2e_empty_vec_len() {
    assert_aelys_int("let v = Vec[]; v.len()", 0);
}

#[test]
fn test_e2e_empty_array_len() {
    assert_aelys_int("let arr = []; arr.len()", 0);
}

#[test]
fn test_e2e_vec_pop_returns_correct_type() {
    assert_aelys_int(
        r#"
        let v = Vec[1, 2, 3];
        let x = v.pop();
        x * 10
        "#,
        30,
    );
}

#[test]
fn test_e2e_array_len_in_expression() {
    assert_aelys_int("let arr = [1, 2, 3, 4, 5]; arr.len() * 2", 10);
}

#[test]
fn test_e2e_vec_len_in_condition() {
    assert_aelys_int(
        r#"
        let v = Vec[1, 2, 3];
        if v.len() > 2 { 100 } else { 0 }
        "#,
        100,
    );
}
