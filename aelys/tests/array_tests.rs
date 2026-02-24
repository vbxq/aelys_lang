mod common;

use aelys_bytecode::{AelysArray, AelysVec, ArrayData, TypeTag, Value};
use common::{
    assert_aelys_bool, assert_aelys_error_contains, assert_aelys_int, run_aelys,
    run_aelys_ok,
};

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
                s += a[i];
                i++;
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
fn test_e2e_vec_passed_to_function() {
    assert_aelys_int(
        r#"
        fn double_all(v) {
            let mut i = 0;
            while i < v.len() {
                v[i] *= 2;
                i++;
            }
        }
        let v = Vec[1, 2, 3];
        double_all(v);
        v[0] + v[1] + v[2]
        "#,
        12, // 2+4+6
    );
}














// Regression tests: empty Vec[] / Array[] must work with non-int types







// Multidimensional array tests

#[test]
fn test_e2e_2d_array_basic() {
    // Create a 2D array (array of arrays)
    assert_aelys_int(
        r#"
        let matrix = [[1, 2, 3], [4, 5, 6], [7, 8, 9]];
        matrix[0][0]
        "#,
        1,
    );
}

#[test]
fn test_e2e_2d_array_access() {
    // Access different elements
    assert_aelys_int("let m = [[1, 2], [3, 4]]; m[0][1]", 2);
    assert_aelys_int("let m = [[1, 2], [3, 4]]; m[1][0]", 3);
    assert_aelys_int("let m = [[1, 2], [3, 4]]; m[1][1]", 4);
}

#[test]
fn test_e2e_2d_array_write() {
    // Modify elements in 2D array
    assert_aelys_int(
        r#"
        let m = [[1, 2], [3, 4]];
        m[0][1] = 99;
        m[0][1]
        "#,
        99,
    );
}

#[test]
fn test_e2e_2d_array_row_access() {
    // Access a row (which is itself an array)
    assert_aelys_int(
        r#"
        let matrix = [[10, 20, 30], [40, 50, 60]];
        let row = matrix[1];
        row[2]
        "#,
        60,
    );
}

#[test]
fn test_e2e_2d_array_sum() {
    // Sum all elements in a 2x3 matrix
    assert_aelys_int(
        r#"
        let m = [[1, 2, 3], [4, 5, 6]];
        let mut sum = 0;
        let mut i = 0;
        while i < 2 {
            let mut j = 0;
            while j < 3 {
                sum += m[i][j];
                j++;
            }
            i++;
        }
        sum
        "#,
        21, // 1+2+3+4+5+6
    );
}

#[test]
fn test_e2e_2d_vec_basic() {
    // Vec of vecs
    assert_aelys_int(
        r#"
        let grid = Vec[Vec[1, 2], Vec[3, 4]];
        grid[0][1]
        "#,
        2,
    );
}


#[test]
fn test_e2e_2d_vec_modify() {
    // Modify elements in vec of vecs
    assert_aelys_int(
        r#"
        let grid = Vec[Vec[1, 2], Vec[3, 4]];
        grid[1][0] = 100;
        grid[1][0]
        "#,
        100,
    );
}

#[test]
fn test_e2e_3d_array() {
    // 3D array access
    assert_aelys_int(
        r#"
        let cube = [[[1, 2], [3, 4]], [[5, 6], [7, 8]]];
        cube[1][0][1]
        "#,
        6, // cube[1][0][1] = 6
    );
}

#[test]
fn test_e2e_3d_array_write() {
    // Modify 3D array element
    assert_aelys_int(
        r#"
        let cube = [[[1, 2], [3, 4]], [[5, 6], [7, 8]]];
        cube[0][1][0] = 999;
        cube[0][1][0]
        "#,
        999,
    );
}

#[test]
fn test_e2e_2d_array_in_function() {
    // Pass 2D array to function
    assert_aelys_int(
        r#"
        fn get_element(matrix, row, col) {
            return matrix[row][col]
        }
        let m = [[10, 20, 30], [40, 50, 60]];
        get_element(m, 1, 2)
        "#,
        60,
    );
}

#[test]
fn test_e2e_2d_array_transpose() {
    // Simple 2x2 matrix transpose
    assert_aelys_int(
        r#"
        let m = [[1, 2], [3, 4]];
        let t = [[0, 0], [0, 0]];
        let mut i = 0;
        while i < 2 {
            let mut j = 0;
            while j < 2 {
                t[j][i] = m[i][j];
                j++;
            }
            i++;
        }
        t[0][1] + t[1][0]
        "#,
        5, // t[0][1]=3, t[1][0]=2, sum=5
    );
}

#[test]
fn test_e2e_mixed_array_vec() {
    // Array of vecs
    assert_aelys_int(
        r#"
        let data = [Vec[1, 2], Vec[3, 4, 5]];
        data[1][2]
        "#,
        5,
    );
}

#[test]
fn test_e2e_vec_of_arrays() {
    // Vec containing arrays
    assert_aelys_int(
        r#"
        let data = Vec[[1, 2], [3, 4]];
        data[0][1]
        "#,
        2,
    );
}

#[test]
fn test_e2e_2d_array_float() {
    // 2D float array
    let result = run_aelys(
        r#"
        let m = [[1.0, 2.0], [3.0, 4.0]];
        m[0][1] + m[1][1]
        "#,
    );
    assert_eq!(result.as_float(), Some(6.0));
}

#[test]
fn test_e2e_2d_array_bool() {
    // 2D boolean array
    assert_aelys_bool(
        r#"
        let grid = [[true, false], [false, true]];
        grid[0][0]
        "#,
        true,
    );
}


#[test]
fn test_e2e_2d_array_computed_index() {
    // Use computed indices
    assert_aelys_int(
        r#"
        let m = [[1, 2, 3], [4, 5, 6], [7, 8, 9]];
        let i = 1;
        let j = 2;
        m[i][j]
        "#,
        6,
    );
}

#[test]
fn test_e2e_2d_array_nested_computed() {
    // Use array value as index for another array
    assert_aelys_int(
        r#"
        let indices = [[0, 1], [1, 0]];
        let data = [[10, 20], [30, 40]];
        let row = indices[0][1];
        let col = indices[1][0];
        data[row][col]
        "#,
        40, // indices[0][1]=1, indices[1][0]=1, so data[1][1]=40
    );
}

#[test]
fn test_e2e_2d_array_find_max() {
    // Find max element in 2D array
    assert_aelys_int(
        r#"
        let m = [[5, 2, 8], [1, 9, 3], [4, 6, 7]];
        let mut max = m[0][0];
        let mut i = 0;
        while i < 3 {
            let mut j = 0;
            while j < 3 {
                if m[i][j] > max {
                    max = m[i][j];
                }
                j++;
            }
            i++;
        }
        max
        "#,
        9,
    );
}

