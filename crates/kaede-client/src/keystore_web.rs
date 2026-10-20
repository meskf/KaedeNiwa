//! WebCrypto keystore：使用 IndexedDB + SubtleCrypto 保护 MLS snapshot 密钥。
//! 仅在 wasm32 平台编译。

#![cfg(target_arch = "wasm32")]

use js_sys::{Array, Object, Reflect, Uint8Array};
use rexie::{ObjectStore, Rexie, TransactionMode};
use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{CryptoKey, SubtleCrypto};

thread_local! {
    static CACHED_DEK: RefCell<Option<[u8; 32]>> = RefCell::new(None);
}

/// 从缓存获取 DEK。
pub fn cached_dek() -> Option<[u8; 32]> {
    CACHED_DEK.with(|c| *c.borrow())
}

/// 初始化 WebCrypto 环境，解锁 DEK 并缓存到内存。
pub async fn init_web_keystore() -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let crypto = window.crypto().map_err(|_| "no crypto")?;
    let subtle = crypto.subtle();

    let rexie = Rexie::builder("kaede-keystore")
        .version(1)
        .add_object_store(ObjectStore::new("keys"))
        .build()
        .await
        .map_err(|e| format!("rexie open: {:?}", e))?;

    // 1. 尝试取 KEK
    let tx = rexie
        .transaction(&["keys"], TransactionMode::ReadWrite)
        .map_err(|e| format!("tx: {:?}", e))?;
    let store = tx.store("keys").map_err(|e| format!("store: {:?}", e))?;
    let kek_req = store
        .get(&JsValue::from_str("kek"))
        .await
        .map_err(|e| format!("get kek: {:?}", e))?;

    let kek: CryptoKey = if kek_req.is_undefined() {
        let algo = Object::new();
        Reflect::set(&algo, &"name".into(), &"AES-GCM".into()).unwrap();
        Reflect::set(&algo, &"length".into(), &JsValue::from_f64(256.0)).unwrap();

        let usages = Array::new();
        usages.push(&"encrypt".into());
        usages.push(&"decrypt".into());

        let key_promise = subtle
            .generate_key_with_object(&algo, false, &usages)
            .map_err(|e| format!("promise: {:?}", e))?;
        let key_js = JsFuture::from(key_promise)
            .await
            .map_err(|e| format!("generate_key: {:?}", e))?;
        let new_kek: CryptoKey = key_js.dyn_into().unwrap();

        store
            .put(&new_kek, Some(&JsValue::from_str("kek")))
            .await
            .map_err(|e| format!("put kek: {:?}", e))?;
        new_kek
    } else {
        let kek: CryptoKey = kek_req.dyn_into().unwrap();
        kek
    };
    tx.done().await.map_err(|e| format!("tx done: {:?}", e))?;

    // 2. 尝试取 DEK 密文
    let tx = rexie
        .transaction(&["keys"], TransactionMode::ReadOnly)
        .map_err(|e| format!("tx: {:?}", e))?;
    let store = tx.store("keys").map_err(|e| format!("store: {:?}", e))?;
    let dek_req = store
        .get(&JsValue::from_str("dek"))
        .await
        .map_err(|e| format!("get dek: {:?}", e))?;

    if !dek_req.is_undefined() {
        let dek_obj = dek_req.dyn_into::<js_sys::Object>().unwrap();
        let iv = Reflect::get(&dek_obj, &"iv".into()).unwrap();
        let ct = Reflect::get(&dek_obj, &"ct".into()).unwrap();

        let algo = Object::new();
        Reflect::set(&algo, &"name".into(), &"AES-GCM".into()).unwrap();
        Reflect::set(&algo, &"iv".into(), &iv).unwrap();

        let decrypt_promise = subtle
            .decrypt_with_object_and_buffer_source(&algo, &kek, &ct.into())
            .map_err(|e| format!("dec promise: {:?}", e))?;
        let dek_bytes = JsFuture::from(decrypt_promise)
            .await
            .map_err(|e| format!("decrypt: {:?}", e))?;
        let mut dek = [0u8; 32];
        dek.copy_from_slice(&Uint8Array::new(&dek_bytes).to_vec());

        CACHED_DEK.with(|c| *c.borrow_mut() = Some(dek));
        return Ok(());
    }

    // 3. 如果没有 DEK 密文，生成新 DEK 并加密存入
    let mut dek = [0u8; 32];
    getrandom::getrandom(&mut dek).map_err(|e| format!("rand: {:?}", e))?;

    let mut iv = [0u8; 12];
    getrandom::getrandom(&mut iv).unwrap();
    let iv_js = Uint8Array::from(&iv[..]);

    let algo = Object::new();
    Reflect::set(&algo, &"name".into(), &"AES-GCM".into()).unwrap();
    Reflect::set(&algo, &"iv".into(), &iv_js).unwrap();

    let dek_js = Uint8Array::from(&dek[..]);
    let encrypt_promise = subtle
        .encrypt_with_object_and_buffer_source(&algo, &kek, &dek_js)
        .map_err(|e| format!("enc promise: {:?}", e))?;
    let ciphertext = JsFuture::from(encrypt_promise)
        .await
        .map_err(|e| format!("encrypt: {:?}", e))?;

    let obj = Object::new();
    Reflect::set(&obj, &"iv".into(), &iv_js).unwrap();
    Reflect::set(&obj, &"ct".into(), &ciphertext).unwrap();

    let tx = rexie
        .transaction(&["keys"], TransactionMode::ReadWrite)
        .map_err(|e| format!("tx: {:?}", e))?;
    let store = tx.store("keys").map_err(|e| format!("store: {:?}", e))?;
    store
        .put(&obj, Some(&JsValue::from_str("dek")))
        .await
        .map_err(|e| format!("put dek: {:?}", e))?;
    tx.done().await.map_err(|e| format!("tx done: {:?}", e))?;

    CACHED_DEK.with(|c| *c.borrow_mut() = Some(dek));
    Ok(())
}
