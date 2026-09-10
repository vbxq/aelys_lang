; ModuleID = 'base'
source_filename = "base"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-pc-linux-gnu"

%Holder = type { i64 }
%__aelys_enum_Tagged = type { i32, [8 x i8] }
%__closure_env___lambda_6 = type { i64 }

@__aelys_global_g = internal global i64 3
@__aelys_rc_type_table = constant [3 x i32] [i32 1, i32 0, i32 3]
@str_6_0 = private constant [25 x i8] c"null pointer dereference\00"
@str_5_0 = private constant [5 x i8] c"base\00"
@str_5_1 = private constant [2 x i8] c"\0A\00"
@str_5_2 = private constant [25 x i8] c"null pointer dereference\00"

define fastcc i64 @helper(ptr %0, i64 %1) {
bb0:
  %iadd = add i64 %1, 1
  ret i64 %iadd
}

define fastcc %Holder @wrap(ptr %0, i64 %1) {
bb0:
  %struct_tmp = alloca %Holder, align 8
  %field_ptr = getelementptr inbounds %Holder, ptr %struct_tmp, i32 0, i32 0
  store i64 %1, ptr %field_ptr, align 8
  %struct_value = load %Holder, ptr %struct_tmp, align 8
  ret %Holder %struct_value
}

define fastcc i64 @unwrap(ptr %0, %__aelys_enum_Tagged %1) {
bb4:
  %l1 = alloca i64, align 8
  %l3 = alloca i64, align 8
  %match_enum_tmp = alloca %__aelys_enum_Tagged, align 4
  store %__aelys_enum_Tagged %1, ptr %match_enum_tmp, align 4
  %match_tag_ptr = getelementptr inbounds %__aelys_enum_Tagged, ptr %match_enum_tmp, i32 0, i32 0
  %match_tag = load i32, ptr %match_tag_ptr, align 4
  switch i32 %match_tag, label %bb3 [
    i32 1, label %bb1
    i32 0, label %bb2
  ]

bb1:                                              ; preds = %bb4
  %match_payload_tmp = alloca %__aelys_enum_Tagged, align 4
  store %__aelys_enum_Tagged %1, ptr %match_payload_tmp, align 4
  %match_payload_base = getelementptr inbounds %__aelys_enum_Tagged, ptr %match_payload_tmp, i32 0, i32 1
  %match_field_0_ptr = getelementptr inbounds i8, ptr %match_payload_base, i32 0
  %match_field_0 = load i64, ptr %match_field_0_ptr, align 4
  store i64 %match_field_0, ptr %l3, align 8
  %ld3 = load i64, ptr %l3, align 8
  store i64 %ld3, ptr %l1, align 8
  br label %bb0

bb2:                                              ; preds = %bb4
  store i64 0, ptr %l1, align 8
  br label %bb0

bb3:                                              ; preds = %bb4
  unreachable

bb0:                                              ; preds = %bb2, %bb1
  %ld1 = load i64, ptr %l1, align 8
  ret i64 %ld1
}

define fastcc i64 @seven(ptr %0) {
bb0:
  ret i64 7
}

define fastcc i64 @__lambda_6(ptr %0, i64 %1) {
bb0:
  %l0 = alloca ptr, align 8
  %l1 = alloca ptr, align 8
  store ptr %0, ptr %l0, align 8
  %ld0 = load ptr, ptr %l0, align 8
  %deref_null_cmp = icmp eq ptr %ld0, null
  br i1 %deref_null_cmp, label %deref_null, label %deref_ok

deref_null:                                       ; preds = %bb0
  call void @__aelys_panic(ptr @str_6_0, i64 24)
  unreachable

deref_ok:                                         ; preds = %bb0
  %place_field = getelementptr inbounds %__closure_env___lambda_6, ptr %ld0, i32 0, i32 0
  store ptr %place_field, ptr %l1, align 8
  %ld1 = load ptr, ptr %l1, align 8
  %deref_null_cmp1 = icmp eq ptr %ld1, null
  br i1 %deref_null_cmp1, label %deref_null2, label %deref_ok3

deref_null2:                                      ; preds = %deref_ok
  call void @__aelys_panic(ptr @str_6_0, i64 24)
  unreachable

deref_ok3:                                        ; preds = %deref_ok
  %deref = load i64, ptr %ld1, align 8
  %iadd = add i64 %1, %deref
  %global_load = load i64, ptr @__aelys_global_g, align 8
  %iadd4 = add i64 %iadd, %global_load
  ret i64 %iadd4
}

define fastcc i64 @__aelys_main(ptr %0) {
bb0:
  %l0 = alloca i64, align 8
  %l2 = alloca { ptr, ptr }, align 8
  %l3 = alloca ptr, align 8
  %l5 = alloca { ptr, ptr }, align 8
  %l8 = alloca %Holder, align 8
  call void @__aelys_write(ptr @str_5_0, i64 4)
  call void @__aelys_write(ptr @str_5_1, i64 1)
  store i64 10, ptr %l0, align 8
  store { ptr, ptr } { ptr @seven, ptr null }, ptr %l2, align 8
  %alloc_raw = call ptr @__aelys_alloc(i64 8)
  store ptr %alloc_raw, ptr %l3, align 8
  %ld0 = load i64, ptr %l0, align 8
  %ld3 = load ptr, ptr %l3, align 8
  %deref_null_cmp = icmp eq ptr %ld3, null
  br i1 %deref_null_cmp, label %deref_null, label %deref_ok

deref_null:                                       ; preds = %bb0
  call void @__aelys_panic(ptr @str_5_2, i64 24)
  unreachable

deref_ok:                                         ; preds = %bb0
  %place_field = getelementptr inbounds %__closure_env___lambda_6, ptr %ld3, i32 0, i32 0
  store i64 %ld0, ptr %place_field, align 8
  %ld31 = load ptr, ptr %l3, align 8
  %closure_env = insertvalue { ptr, ptr } { ptr @__lambda_6, ptr undef }, ptr %ld31, 1
  store { ptr, ptr } %closure_env, ptr %l5, align 8
  %call_direct = call fastcc i64 @helper(ptr null, i64 1)
  %call_direct2 = call fastcc %Holder @wrap(ptr null, i64 %call_direct)
  store %Holder %call_direct2, ptr %l8, align 8
  %field_ptr = getelementptr inbounds %Holder, ptr %l8, i32 0, i32 0
  %field_load = load i64, ptr %field_ptr, align 8
  %enum_tmp = alloca %__aelys_enum_Tagged, align 4
  %enum_tag_ptr = getelementptr inbounds %__aelys_enum_Tagged, ptr %enum_tmp, i32 0, i32 0
  store i32 1, ptr %enum_tag_ptr, align 4
  %enum_payload_ptr = getelementptr inbounds %__aelys_enum_Tagged, ptr %enum_tmp, i32 0, i32 1
  store [8 x i8] zeroinitializer, ptr %enum_payload_ptr, align 1
  %enum_payload_base = getelementptr inbounds %__aelys_enum_Tagged, ptr %enum_tmp, i32 0, i32 1
  %enum_field_0_ptr = getelementptr inbounds i8, ptr %enum_payload_base, i32 0
  store i64 4, ptr %enum_field_0_ptr, align 4
  %enum_value = load %__aelys_enum_Tagged, ptr %enum_tmp, align 4
  %call_direct3 = call fastcc i64 @unwrap(ptr null, %__aelys_enum_Tagged %enum_value)
  %iadd = add i64 %field_load, %call_direct3
  %ld2 = load { ptr, ptr }, ptr %l2, align 8
  %closure_fn = extractvalue { ptr, ptr } %ld2, 0
  %closure_env4 = extractvalue { ptr, ptr } %ld2, 1
  %call_closure = call fastcc i64 %closure_fn(ptr %closure_env4)
  %iadd5 = add i64 %iadd, %call_closure
  %ld5 = load { ptr, ptr }, ptr %l5, align 8
  %closure_fn6 = extractvalue { ptr, ptr } %ld5, 0
  %closure_env7 = extractvalue { ptr, ptr } %ld5, 1
  %call_closure8 = call fastcc i64 %closure_fn6(ptr %closure_env7, i64 2)
  %iadd9 = add i64 %iadd5, %call_closure8
  %call_direct10 = call fastcc i64 @__mono_ident_i64(ptr null, i64 6)
  %iadd11 = add i64 %iadd9, %call_direct10
  ret i64 %iadd11
}

define fastcc i64 @__mono_ident_i64(ptr %0, i64 %1) {
bb0:
  ret i64 %1
}

; Function Attrs: noreturn
declare void @__aelys_panic(ptr, i64) #0

declare void @__aelys_write(ptr, i64)

declare ptr @__aelys_alloc(i64)

define i64 @__aelys_user_main() {
entry:
  %user_main = call fastcc i64 @__aelys_main(ptr null)
  ret i64 %user_main
}

attributes #0 = { noreturn }
