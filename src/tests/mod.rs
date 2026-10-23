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
    fn options_never_take_their_value_as_a_separate_argument() {
        // mpv 只认 `--opt=value`（外加单连字符的 legacy 形式）。`--opt value` 会让它报
        // "option requires parameter" 然后 `Exiting... (Fatal error)`，**在建窗口之前**就退出——
        // 第十八轮真机上"mpv 窗口没出现"正是这个。除布尔开关外一律要带 `=`。
        let args = build_arguments(
            "https://v/v.m4s",
            Some("https://a/a.m4s"),
            &["Cookie: SESSDATA=x".to_string()],
        );
        for arg in &args[..args.len() - 1] {
            if arg.starts_with("--") && !arg.contains('=') {
                assert!(
                    matches!(arg.as_str(), "--no-config" | "--force-window"),
                    "带值的选项必须写成 --opt=value：{arg}"
                );
            }
        }
        // 只给所有构建都存在的选项：`--no-ytdl` 在 `#if HAVE_LUA` 里，不带 Lua 的 mpv
        // 会把它当未知选项，症状又是"窗口不出现"。
        assert!(
            !args.iter().any(|arg| arg.contains("ytdl")),
            "{args:?}"
        );
        // 三个真正带值的网络选项确认是拼接形式。
        assert!(args
            .iter()
            .any(|arg| arg.starts_with("--user-agent=Mozilla/5.0")));
        assert!(args
            .iter()
            .any(|arg| arg == &format!("--referrer={BILIBILI_REFERER}")));
    }

    #[test]
    fn video_url_is_the_last_positional_argument() {
        let args = build_arguments(
            "https://v/video.m4s",
            Some("https://a/audio.m4s"),
            &[],
        );
        assert_eq!(args.last().map(String::as_str), Some("https://v/video.m4s"));
    }

    #[test]
    fn separate_audio_stream_is_passed_as_external_track() {
        // B 站音视频是两条 DASH 流，音频只能靠 --audio-file 附加。
        let args = build_arguments(
            "https://v/video.m4s",
            Some("https://a/audio.m4s"),
            &[],
        );
        assert!(args
            .iter()
            .any(|arg| arg == "--audio-file=https://a/audio.m4s"));
    }

    #[test]
    fn single_file_playback_adds_no_audio_argument() {
        // durl 回退时文件里已经含音频；这时再给一个空的 --audio-file= 会让 mpv 直接报错。
        let args = build_arguments("https://v/video.mp4", None, &[]);
        assert!(
            !args.iter().any(|arg| arg.starts_with("--audio-file")),
            "{args:?}"
        );
        assert_eq!(args.last().map(String::as_str), Some("https://v/video.mp4"));
    }

    #[test]
    fn referer_header_is_present_and_global() {
        let args = build_arguments(
            "https://v/v.m4s",
            Some("https://a/a.m4s"),
            &[],
        );
        // 多条头是拼在**同一个** `--http-header-fields=` 里的（列表选项按逗号切分，
        // 而重复给同一选项是替换，见 player 模块注释）。
        let joined = args
            .iter()
            .find(|arg| arg.starts_with("--http-header-fields="))
            .expect("应当显式传入 http 头");
        assert!(joined.contains(&format!("Referer: {BILIBILI_REFERER}")), "{joined}");
    }

    #[test]
    fn mpv_is_resolved_from_the_run_directory_not_path() {
        // 定稿：只认 `<exe 同目录>/mpv.exe`。PATH 那条路会先命中控制台版 mpv.com，
        // 它再派生的 mpv.exe 不受我们的 kill 管。
        let dir = crate::app::exe_dir();
        let path = crate::player::mpv_path();
        assert_eq!(path.file_name().and_then(|name| name.to_str()), Some("mpv.exe"));
        assert_eq!(path.parent(), Some(dir.as_path()));
    }

    #[test]
    fn extra_headers_are_appended() {
        let args = build_arguments(
            "https://v/v.m4s",
            Some("https://a/a.m4s"),
            &[format!("Cookie: SESSDATA=x")],
        );
        let joined = args.join(" ");
        assert!(joined.contains("Cookie: SESSDATA=x"), "{joined}");
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
    use crate::api::login::{QrPollState, credential_from_cookies, needs_buvid3_backfill};
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

    #[test]
    fn only_a_successful_poll_without_buvid3_needs_backfill() {
        // 真机事实：B 站扫码成功的响应里只有登录三件套，没有 buvid3，
        // 而 `Account::validate_complete` 要它 —— 所以这种组合必须触发补取。
        let without_buvid3: Vec<(String, String)> = full_cookies()
            .into_iter()
            .filter(|(key, _)| key != "buvid3")
            .collect();
        assert!(needs_buvid3_backfill(0, &without_buvid3));
        // 齐了就不用补；没成功（等待扫码 86101 / 已过期 86038）也不该补，免得白发消息。
        assert!(!needs_buvid3_backfill(0, &full_cookies()));
        assert!(!needs_buvid3_backfill(86101, &[]));
        assert!(!needs_buvid3_backfill(86038, &without_buvid3));
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
mod mine_tests {
    use crate::api::format_seconds;
    use crate::api::profile::vip_label;

    #[test]
    fn seconds_format_switches_at_one_hour() {
        // 卡片右下角那一栏同时出现"mm:ss"和"h:mm:ss"两种写法，边界要钉住。
        assert_eq!(format_seconds(0), "00:00");
        assert_eq!(format_seconds(45), "00:45");
        assert_eq!(format_seconds(599), "09:59");
        assert_eq!(format_seconds(3600), "1:00:00");
        assert_eq!(format_seconds(3725), "1:02:05");
        assert_eq!(format_seconds(86_399), "23:59:59");
    }

    #[test]
    fn vip_label_only_cares_about_active_status()
    {
        // 未知/未开通一律不显示会员；status==0 或 kind==0 都算没有（口径见 api/profile.rs）。
        assert_eq!(vip_label(0, 0), "");
        assert_eq!(vip_label(1, 0), "");
        assert_eq!(vip_label(0, 2), "");
        assert_eq!(vip_label(1, 1), "大会员");
        assert_eq!(vip_label(1, 2), "大会员");
    }
}

#[cfg(test)]
mod cover_tests {
    use crate::ui::cover;

    #[test]
    fn protocol_relative_url_becomes_loadable() {
        // 第十五轮真机日志里的原案：搜索接口给的是 `//i2.hdslb.com/…`，
        // gpui 用 `Url::parse` 判 URL，这种写法会被当成内置资源而永远加载不出来。
        let raw = "//i2.hdslb.com/bfs/archive/538e997a97552c30766137327a1b1161437200e6.jpg";
        assert_eq!(
            cover::canonical(raw),
            "https://i2.hdslb.com/bfs/archive/538e997a97552c30766137327a1b1161437200e6.jpg"
        );
        // 显示用的最终地址：补协议 + 按卡片框要缩略图。
        assert_eq!(
            cover::for_display(raw, 320, 180),
            "https://i2.hdslb.com/bfs/archive/538e997a97552c30766137327a1b1161437200e6.jpg@320w_180h.jpg"
        );
    }

    #[test]
    fn only_hdslb_gets_https_upgrade_and_thumbnail_suffix() {
        // 图床的 http 地址升 https（实测两种都回 200）。
        assert_eq!(
            cover::canonical("http://i0.hdslb.com/bfs/archive/a.jpg"),
            "https://i0.hdslb.com/bfs/archive/a.jpg"
        );
        // 别的域名一律不动：`@…` 是图床私有语法，拼错了是 400。
        assert_eq!(
            cover::canonical("http://example.com/a.jpg"),
            "http://example.com/a.jpg"
        );
        assert_eq!(
            cover::sized("https://example.com/a.jpg", 320, 180),
            "https://example.com/a.jpg"
        );
        // 已经带 `@` 的不重复拼；GIF 保留动图。
        assert_eq!(
            cover::sized("https://i0.hdslb.com/bfs/archive/a.jpg@320w_180h.jpg", 640, 360),
            "https://i0.hdslb.com/bfs/archive/a.jpg@320w_180h.jpg"
        );
        assert_eq!(
            cover::sized("https://i0.hdslb.com/bfs/emoji/x.gif", 64, 64),
            "https://i0.hdslb.com/bfs/emoji/x.gif"
        );
    }

    #[test]
    fn empty_url_stays_empty() {
        // 空地址在卡片里走占位块分支，绝不能变成 "https://" 这种看起来能用其实必挂的东西。
        assert_eq!(cover::canonical(""), "");
        assert_eq!(cover::for_display("   ", 320, 180), "");
    }
}

#[cfg(test)]
mod detail_tests {
    use crate::api::detail::{favorite_count, normalize_subtitle_url};
    use crate::api::{format_seconds, seconds_to_u32};

    #[test]
    fn favorite_prefers_the_new_field_name() {
        // B 站新旧载荷字段名不同（`favorite` / `fav`），只读一个会显示 0。
        assert_eq!(favorite_count(Some(7), Some(3)), 7);
        assert_eq!(favorite_count(None, Some(3)), 3);
        assert_eq!(favorite_count(Some(0), None), 0);
        assert_eq!(favorite_count(None, None), 0);
    }

    #[test]
    fn subtitle_url_gets_a_scheme() {
        // 字幕地址和封面一样是协议相对的，mpv 的 `sub-add` 要完整 URL。
        assert_eq!(
            normalize_subtitle_url("//comment.bilibili.com/1234.json"),
            "https://comment.bilibili.com/1234.json"
        );
        // 已经是绝对地址的不动，前后空白去掉。
        assert_eq!(
            normalize_subtitle_url(" https://s1.hdslb.com/x.json "),
            "https://s1.hdslb.com/x.json"
        );
    }

    #[test]
    fn wide_second_counts_are_clamped_not_wrapped() {
        // `duration` 在接口里是 u64，格式化要的是 u32；超限当上限截断，不该 panic 也不该回绕。
        assert_eq!(seconds_to_u32(u64::from(u32::MAX)), u32::MAX);
        assert_eq!(seconds_to_u32(u32::MAX as u64 + 1), u32::MAX);
        assert_eq!(seconds_to_u32(0), 0);
        // 顺带确认"正好一小时"这一边界在 api 层的格式化函数上。
        assert_eq!(format_seconds(3_600), "1:00:00");
    }
}

#[cfg(test)]
mod playback_tests {
    use crate::api::stream::{quality_label, quality_options};
    use crate::store::credentials::SavedCredential;

    fn support(quality: u64, new_description: &str, display_desc: &str) -> bpi_rs::video::videostream_url::SupportFormat {
        bpi_rs::video::videostream_url::SupportFormat {
            quality,
            format: "mp4".to_string(),
            new_description: new_description.to_string(),
            display_desc: display_desc.to_string(),
            superscript: String::new(),
            codecs: None,
        }
    }

    #[test]
    fn label_prefers_the_new_support_formats_field() {
        let formats = vec![support(80, "1080P 高清", "1080P"), support(64, "720P", "720P")];
        assert_eq!(quality_label(&formats, &[], &[], 80), "1080P 高清");
        // support_formats 里没有这一档时，退回 accept_* 的下标配对。
        let accept_quality = vec![116u64, 80, 64];
        let accept_description = vec!["1080P60".to_string(), "1080P".to_string(), "720P".to_string()];
        assert_eq!(
            quality_label(&formats, &accept_quality, &accept_description, 116),
            "1080P60"
        );
        // 两处都没有：至少把档号写出来，日志看得懂。
        assert_eq!(quality_label(&[], &[], &[], 32), "32P");
    }

    #[test]
    fn support_formats_without_description_fall_back_to_display_desc() {
        let formats = vec![support(64, "", "720P 准高清")];
        assert_eq!(quality_label(&formats, &[], &[], 64), "720P 准高清");
        let options = quality_options(&formats, &[], &[]);
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].label, "720P 准高清");
    }

    #[test]
    fn cookie_header_lists_only_what_we_have() {
        let credential = SavedCredential {
            dede_user_id: "42".to_string(),
            sessdata: "sess".to_string(),
            bili_jct: "csrf".to_string(),
            buvid3: "buvid".to_string(),
            refresh_token: String::new(),
            saved_at_unix: 0,
        };
        assert_eq!(
            credential.cookie_header(),
            "DedeUserID=42; SESSDATA=sess; bili_jct=csrf; buvid3=buvid"
        );

        let partial = SavedCredential {
            sessdata: String::new(),
            ..credential
        };
        assert_eq!(
            partial.cookie_header(),
            "DedeUserID=42; bili_jct=csrf; buvid3=buvid"
        );
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
