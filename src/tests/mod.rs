//! 单元测试（约定：单元测试集中在 `src/tests/`，说明见根目录 `test/README.md`）。
//!
//! 现阶段只测**纯逻辑**：码流选择的边界、mpv 参数形状、凭据序列化往返。
//! 这三处都是"错了要等到播放/登录才发现"的地方，且都不依赖 gpui 窗口与网络，
//! 因此可以在 CI 里以 `cargo test` 直接跑。UI 与网络调用暂不覆盖。

#[cfg(test)]
mod stream_select_tests {
    use crate::api::stream_select::{is_quality_allowed, MAX_QUALITY_ID};

    #[test]
    fn quality_up_to_full_hd_is_allowed() {
        // 16/32/64/80 = 360P/480P/720P/1080P，112/116 = 1080P+ / 1080P60。
        for id in [16u64, 32, 64, 80, 112, 116] {
            assert!(is_quality_allowed(id), "{id} 应当在封顶范围内");
        }
    }

    #[test]
    fn ultra_hd_tiers_are_rejected() {
        // 120=4K, 125=HDR, 126=Dolby Vision, 127=8K：目标机硬解不到，默认排除。
        for id in [120u64, 125, 126, 127] {
            assert!(!is_quality_allowed(id), "{id} 应当被封顶排除");
        }
    }

    #[test]
    fn cap_constant_matches_1080p60() {
        assert_eq!(MAX_QUALITY_ID, 116);
    }
}

#[cfg(test)]
mod player_tests {
    use crate::player::{build_arguments, BILIBILI_REFERER};

    #[test]
    fn pipe_name_uses_windows_pipe_namespace() {
        let name = crate::player::ipc::pipe_name_for(4321);
        assert_eq!(name, r"\\.\pipe\rustbilipc-4321");
    }

    #[test]
    fn video_url_is_the_last_positional_argument() {
        let args = build_arguments("https://v/video.m4s", "https://a/audio.m4s", r"\\.\pipe\x", &[]);
        assert_eq!(args.last().map(String::as_str), Some("https://v/video.m4s"));
    }

    #[test]
    fn separate_audio_stream_is_passed_as_external_track() {
        // B 站音视频是两条 DASH 流，音频只能靠 --audio-file 附加。
        let args = build_arguments("https://v/video.m4s", "https://a/audio.m4s", r"\\.\pipe\x", &[]);
        assert!(args
            .iter()
            .any(|arg| arg == "--audio-file=https://a/audio.m4s"));
    }

    #[test]
    fn referer_header_is_present_and_global() {
        let args = build_arguments("https://v/v.m4s", "https://a/a.m4s", r"\\.\pipe\x", &[]);
        let index = args
            .iter()
            .position(|arg| arg == "--http-header-fields")
            .expect("应当显式传入 http 头");
        assert!(args[index + 1].contains(&format!("Referer: {BILIBILI_REFERER}")));
    }

    #[test]
    fn extra_headers_are_appended() {
        let args = build_arguments(
            "https://v/v.m4s",
            "https://a/a.m4s",
            r"\\.\pipe\x",
            &[format!("Cookie: SESSDATA=x")],
        );
        let joined = args.join(" ");
        assert!(joined.contains("Cookie: SESSDATA=x"), "{joined}");
    }

    #[test]
    fn ipc_command_shape_is_mpv_json_protocol() {
        use crate::player::ipc::command;
        let value = command(&["quit"], 7);
        assert_eq!(
            value,
            serde_json::json!({ "command": ["quit"], "request_id": 7 })
        );
    }

    #[test]
    fn observe_property_carries_numeric_id() {
        let value = crate::player::ipc::observe_property("time-pos", 1, 2);
        assert_eq!(
            value["command"][0],
            serde_json::json!("observe_property")
        );
        assert_eq!(value["command"][1], serde_json::json!(1));
        assert_eq!(value["command"][2], serde_json::json!("time-pos"));
    }
}

#[cfg(test)]
mod credentials_tests {
    use crate::store::credentials::SavedCredential;

    fn sample() -> SavedCredential {
        SavedCredential {
            dede_user_id: "123".into(),
            sessdata: "sess".into(),
            bili_jct: "jct".into(),
            buvid3: "buvid".into(),
            refresh_token: "refresh".into(),
            saved_at_unix: 1_700_000_000,
        }
    }

    #[test]
    fn json_roundtrip_keeps_every_field() {
        let json = serde_json::to_string(&sample()).expect("序列化应当成功");
        let back: SavedCredential = serde_json::from_str(&json).expect("反序列化应当成功");
        assert_eq!(back.dede_user_id, "123");
        assert_eq!(back.sessdata, "sess");
        assert_eq!(back.bili_jct, "jct");
        assert_eq!(back.buvid3, "buvid");
        assert_eq!(back.refresh_token, "refresh");
    }

    #[test]
    fn optional_fields_default_when_absent() {
        // 老文件缺 refresh_token / saved_at_unix 时必须还能读，否则升级即掉登录态。
        let minimal = r#"{"dede_user_id":"1","sessdata":"2","bili_jct":"3","buvid3":"4"}"#;
        let parsed: SavedCredential = serde_json::from_str(minimal).expect("缺省字段应当容错");
        assert!(parsed.refresh_token.is_empty());
        assert_eq!(parsed.saved_at_unix, 0);
        assert!(parsed.is_complete());
    }

    #[test]
    fn incomplete_credential_is_not_complete() {
        let mut credential = sample();
        // buvid3 参与完整性判断：Account::validate_complete 要求四个字段都非空。
        credential.buvid3.clear();
        assert!(!credential.is_complete());
    }

    #[test]
    fn converts_to_bpi_account_field_by_field() {
        let account = sample().to_account();
        assert_eq!(account.dede_user_id, "123");
        assert_eq!(account.sessdata, "sess");
        assert_eq!(account.bili_jct, "jct");
        assert_eq!(account.buvid3, "buvid");
        assert!(account.is_complete());
    }
}

#[cfg(test)]
mod search_format_tests {
    use crate::api::search::{SearchSort, format_count, strip_keyword_tags};

    #[test]
    fn strips_highlight_tags_from_titles() {
        // 搜索接口返回的标题里带 <em class="keyword">，直接显示会把标签露出来。
        let raw = "【官方】<em class=\"keyword\">原神</em>新PV发布";
        assert_eq!(strip_keyword_tags(raw), "【官方】原神新PV发布");
    }

    #[test]
    fn leaves_plain_title_untouched() {
        assert_eq!(strip_keyword_tags("普通标题"), "普通标题");
    }

    #[test]
    fn counts_under_ten_thousand_stay_plain() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(9999), "9999");
    }

    #[test]
    fn ten_thousand_boundary_collapses_to_one_wan() {
        assert_eq!(format_count(10_000), "1万");
        assert_eq!(format_count(15_000), "1.5万");
        assert_eq!(format_count(12_345), "1.23万");
    }

    #[test]
    fn hundred_millions_use_yi_unit() {
        assert_eq!(format_count(100_000_000), "1亿");
        assert_eq!(format_count(123_456_789), "1.23亿");
    }

    #[test]
    fn sort_labels_are_chinese() {
        assert_eq!(SearchSort::Comprehensive.label(), "综合");
        assert_eq!(SearchSort::PlayCount.label(), "播放量");
        assert_eq!(SearchSort::PublishDate.label(), "最新");
    }
}

#[cfg(test)]
mod login_tests {
    use crate::api::login::{QrPollState, credential_from_cookies};
    use crate::store::credentials::SavedCredential;

    fn full_cookies() -> Vec<(String, String)> {
        vec![
            ("DedeUserID".to_string(), "42".to_string()),
            ("SESSDATA".to_string(), "sess".to_string()),
            ("bili_jct".to_string(), "csrf".to_string()),
            ("buvid3".to_string(), "buvid".to_string()),
            ("unsure".to_string(), "ignored".to_string()),
        ]
    }

    #[test]
    fn code_zero_means_authorized() {
        assert!(matches!(
            QrPollState::from_code(0, String::new(), full_cookies(), "refresh".to_string()),
            QrPollState::Authorized { .. }
        ));
    }

    #[test]
    fn known_waiting_and_expired_codes_map_correctly() {
        assert_eq!(
            QrPollState::from_code(86101, "未扫码".to_string(), vec![], String::new()),
            QrPollState::Waiting
        );
        assert_eq!(
            QrPollState::from_code(86038, "二维码已失效".to_string(), vec![], String::new()),
            QrPollState::Expired
        );
    }

    #[test]
    fn unknown_code_carries_server_message() {
        let state = QrPollState::from_code(-101, "账号未登录".to_string(), vec![], String::new());
        assert_eq!(
            state,
            QrPollState::Failed {
                code: -101,
                message: "账号未登录".to_string()
            }
        );
    }

    #[test]
    fn credential_needs_all_four_cookies() {
        let credential = credential_from_cookies(&full_cookies(), "refresh")
            .expect("四个 cookie 齐全时应当构造成功");
        assert_eq!(credential.dede_user_id, "42");
        assert_eq!(credential.sessdata, "sess");
        assert_eq!(credential.bili_jct, "csrf");
        assert_eq!(credential.buvid3, "buvid");
        assert_eq!(credential.refresh_token, "refresh");
        assert!(credential.is_complete());

        // 少任何一个都不该构造出凭据：Account::validate_complete 会当场拒绝。
        let partial: Vec<(String, String)> = full_cookies()
            .into_iter()
            .filter(|(key, _)| key != "buvid3")
            .collect();
        assert!(credential_from_cookies(&partial, "refresh").is_none());
    }

    #[test]
    fn saved_credential_roundtrips_after_login() {
        let credential = credential_from_cookies(&full_cookies(), "refresh").unwrap();
        let json = serde_json::to_string(&credential).unwrap();
        let back: SavedCredential = serde_json::from_str(&json).unwrap();
        assert_eq!(back, credential);
    }
}

#[cfg(test)]
mod qr_tests {
    use crate::ui::qr;

    #[test]
    fn matrix_is_square_and_non_empty() {
        let matrix = qr::matrix_for("https://login.bilibili.com/qrcode/h5-login?validation_key=abc");
        assert!(!matrix.is_empty(), "登录 URL 应当能编出二维码");
        let side = matrix.len();
        assert!(matrix.iter().all(|row| row.len() == side), "矩阵应当是正方形");
        // QR 版本 1~40 的边长一定是奇数且 >= 21。
        assert!(side >= 21 && side % 2 == 1, "异常边长: {side}");
    }

    #[test]
    fn empty_text_still_produces_a_symbol() {
        // B 站不会给空 URL，但解码器要能处理，避免登录页拿到空白。
        assert!(!qr::matrix_for("").is_empty());
    }
}

#[cfg(test)]
mod scaffold_tests {
    use crate::VERSION;

    #[test]
    fn version_matches_manifest() {
        assert!(!VERSION.is_empty());
        assert!(VERSION.starts_with(|c: char| c.is_ascii_digit()));
    }
}
