//! 单元测试（约定：单元测试集中在 `src/tests/`，说明见根目录 `test/README.md`）。
//!
//! 现阶段只测**纯逻辑**：码流选择的边界、mpv 参数形状、凭据序列化往返。
//! 这三处都是"错了要等到播放/登录才发现"的地方，且都不依赖 gpui 窗口与网络，
//! 因此可以在 CI 里以 `cargo test` 直接跑。UI 与网络调用暂不覆盖。

#[cfg(test)]
mod stream_select_tests {
    use crate::api::stream_select::{DEFAULT_QUALITY_CAP, is_quality_allowed, pick_video};
    use crate::store::config::CodecPreference;
    use bpi_rs::video::videostream_url::DashStream;

    #[test]
    fn quality_up_to_full_hd_is_allowed() {
        // 16/32/64/80 = 360P/480P/720P/1080P，112/116 = 1080P+ / 1080P60。
        // 封顶值由调用方给（它来自 config.json 的 max_quality），测试里给默认的 116。
        for id in [16u64, 32, 64, 80, 112, 116] {
            assert!(
                is_quality_allowed(id, DEFAULT_QUALITY_CAP),
                "{id} 应当在封顶范围内"
            );
        }
    }

    #[test]
    fn tiers_above_the_cap_are_rejected() {
        // 120=4K, 125=HDR, 126=Dolby Vision, 127=8K：**默认**封顶之外（目标机硬解不到）。
        // P13 起用户能在设置页把上限调到 127，所以这里测的是"给定 cap 时 behaved"，
        // 而不是"这些档永远不许"。
        for id in [120u64, 125, 126, 127] {
            assert!(
                !is_quality_allowed(id, DEFAULT_QUALITY_CAP),
                "{id} 应当被默认封顶排除"
            );
            assert!(is_quality_allowed(id, 127), "{id} 应当在 8K 封顶以内");
        }
        // 把封顶调低一档，116 自己也要被排除——这条证明"参数真的生效"而不是恒等于常量。
        assert!(!is_quality_allowed(116, 64));
        assert!(is_quality_allowed(64, 64));
    }

    #[test]
    fn cap_constant_matches_1080p60() {
        assert_eq!(DEFAULT_QUALITY_CAP, 116);
    }

    /// 造一条流：选择只用得到 `id`/`codecs`/`bandwidth` 三个键，其余字段一律给"没有"。
    /// （`DashStream` 的字段都是公开的、没有 `Default`，所以这里逐个写全——
    /// 以后库里加字段会在这里编译失败，那是**好事**：说明选择逻辑可能也要跟着看一眼。）
    fn stream(id: u64, codecs: &str, bandwidth: u64) -> DashStream {
        DashStream {
            id,
            base_url: String::new(),
            backup_url: Vec::new(),
            bandwidth,
            mime_type: "video/mp4".to_string(),
            codecs: codecs.to_string(),
            width: None,
            height: None,
            frame_rate: None,
            sar: None,
            start_with_sap: None,
            segment_base: None,
            md5: None,
            size: None,
            db_type: None,
            r#type: None,
            stream_name: None,
            orientation: None,
        }
    }

    #[test]
    fn default_preference_still_prefers_hevc_over_av1_and_avc() {
        // 默认偏好 HEVC：同画质档里 HEVC > AV1 > AVC（省带宽优先）。
        let streams = vec![
            stream(80, "avc1.64.00", 9_000_000),
            stream(80, "av01.0.08M.08", 8_000_000),
            stream(80, "hev1.1.6.L150.90", 7_000_000),
        ];
        let picked = pick_video(&streams, 116, CodecPreference::Hevc).expect("应当选出一路");
        assert!(picked.codecs.starts_with("hev1"), "{}", picked.codecs);
    }

    #[test]
    fn explicit_codec_preference_beats_the_default_order() {
        // 他选了 AV1 就该给 AV1，哪怕同档里 HEVC 更省带宽——
        // 否则界面上写着 AV1、实际播的是 HEVC，就是假设置。
        let streams = vec![
            stream(80, "hev1.1.6.L150.90", 9_000_000),
            stream(80, "av01.0.08M.08", 7_000_000),
        ];
        let av1 = pick_video(&streams, 116, CodecPreference::Av1).expect("应当选出一路");
        assert!(av1.codecs.starts_with("av01"), "{}", av1.codecs);
        let avc_missing = pick_video(&streams, 116, CodecPreference::Avc).expect("应当选出一路");
        // 这一档没有 avc1：偏好那一类不存在时**退回默认顺序**（HEVC 在前），不能选不出来。
        assert!(avc_missing.codecs.starts_with("hev1"), "{}", avc_missing.codecs);
    }

    #[test]
    fn quality_still_outranks_codec_preference() {
        // 编码偏好只在**同一画质档内**决定谁先；不许为了 HEVC 放弃更高的档。
        let streams = vec![
            stream(116, "avc1.64.00", 5_000_000),
            stream(80, "hev1.1.6.L150.90", 9_000_000),
        ];
        let picked = pick_video(&streams, 116, CodecPreference::Hevc).expect("应当选出一路");
        assert_eq!(picked.id, 116);
        assert!(picked.codecs.starts_with("avc1"), "{}", picked.codecs);
    }

    #[test]
    fn empty_candidates_have_no_pick() {
        // 空列表不是错误，是"没有流可选"：由上层（`api::stream`）翻成一句给人看的话。
        assert!(pick_video(&[], 116, CodecPreference::Hevc).is_none());
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
    fn fallback_location_is_still_the_run_directory() {
        // 自动查找的最后一级兜底仍然是 `<exe 同目录>/mpv.exe`（他现在那份 publish/mpv.exe 还能用）。
        let dir = crate::app::exe_dir();
        let path = crate::player::mpv_path();
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(crate::player::exe_name())
        );
        assert_eq!(path.parent(), Some(dir.as_path()));
    }

    #[test]
    fn mpv_resolution_prefers_path_over_run_directory() {
        // P13 的定稿顺序：设置里指定的 → PATH → 运行目录。用注入的 `exists` 判定，不碰真文件系统。
        use std::path::{Path, PathBuf};
        let dirs = vec![PathBuf::from("pathx")];
        let exe = crate::player::exe_name();

        // 现场一：PATH 里没有，运行目录里有 → 兜底那份。
        let run = Path::new("run").join(exe);
        let picked = crate::player::resolve_mpv("", &dirs, Path::new("run"), &|path: &Path| {
            path == run.as_path()
        });
        assert_eq!(picked, Some(run), "运行目录那份该被当成兜底");

        // 现场二：PATH 里有 → 用 PATH 那份（它排在运行目录之前）。
        let from_path = Path::new("pathx").join(exe);
        let picked = crate::player::resolve_mpv("", &dirs, Path::new("run"), &|path: &Path| {
            path == from_path.as_path()
        });
        assert_eq!(picked, Some(from_path));
    }

    #[test]
    fn mpv_lookup_only_asks_for_the_exe_never_the_console_launcher() {
        // 第十八轮真机的坑：`PATHEXT` 把 `.COM` 排在 `.EXE` 前，按 shell 语义找会先命中
        // `mpv.com`——那是个会再派生一个 mpv.exe 的控制台启动器，我们 kill 不到真正在放的那个。
        // `resolve_mpv` 自己拼 `目录/mpv.exe` 去判存在，所以就算两种都在也只可能选到 exe。
        use std::path::{Path, PathBuf};
        let both = |path: &Path| -> bool {
            let text = path.to_string_lossy().to_string();
            text.ends_with("mpv.exe") || text.ends_with("mpv.com")
        };
        let picked = crate::player::resolve_mpv(
            "",
            &[PathBuf::from("pathx")],
            Path::new("run"),
            &both,
        );
        let picked = picked.expect("PATH 里那份该被选中");
        assert!(
            picked.to_string_lossy().ends_with(crate::player::exe_name()),
            "{picked:?}"
        );
        assert!(
            !picked.to_string_lossy().ends_with(".com"),
            "绝不接受 mpv.com：{picked:?}"
        );
    }

    #[test]
    fn explicit_mpv_path_wins_and_a_broken_one_falls_through() {
        use std::path::{Path, PathBuf};
        let good = Path::new("pathx").join(crate::player::exe_name());
        // 显式路径存在 → 用它（哪怕 PATH 与运行目录也各有一份）。
        let picked = crate::player::resolve_mpv(
            good.to_str().unwrap_or_default(),
            &[PathBuf::from("pathx")],
            Path::new("run"),
            &|path: &Path| path == good.as_path(),
        );
        assert_eq!(picked, Some(good.clone()));
        // 显式路径读不到 → 继续往下找，并**不退化成"随便跑个什么"**。
        let fell = crate::player::resolve_mpv(
            "settings/没有这个文件.exe",
            &[PathBuf::from("pathx")],
            Path::new("run"),
            &|path: &Path| path == good.as_path(),
        );
        assert_eq!(fell, Some(good));
        // 三处都没有 → None，由上层给一句实话（设置页那行与起播失败的原因都读这个）。
        let none = crate::player::resolve_mpv("", &[], Path::new("run"), &|_path: &Path| false);
        assert_eq!(none, None);
    }

    #[test]
    fn path_variable_splits_without_empty_entries() {
        // `path_directories` 会把空段滤掉：空段在 shell 语义里等于"当前目录"，
        // 那会让程序在用户的当前目录里找一个 mpv.exe——最坏的"随机执行"。
        // 手工拼分隔符而不是 `env::join_paths`：后者在 Windows 上遇到空元素**直接返回 Err**，
        // 而我们要测的恰好就是"字符串里有一个空段"。
        let separator = if cfg!(windows) { ";" } else { ":" };
        let joined = ["/opt/tools/bin", "", "/usr/bin"].join(separator);
        let dirs = crate::player::path_directories(&joined);
        assert_eq!(dirs.len(), 2, "{dirs:?}");
        assert!(dirs.iter().all(|one| !one.as_os_str().is_empty()));
        assert!(crate::player::path_directories("").is_empty());
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
mod auth_tests {
    use crate::store::auth::Auth;

    fn sample() -> Auth {
        Auth {
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
        let back: Auth = serde_json::from_str(&json).expect("反序列化应当成功");
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
        let parsed: Auth = serde_json::from_str(minimal).expect("缺省字段应当容错");
        assert!(parsed.refresh_token.is_empty());
        assert_eq!(parsed.saved_at_unix, 0);
        assert!(parsed.is_complete());
    }

    #[test]
    fn broken_auth_text_is_treated_as_not_logged_in() {
        // 这是"读不到就当没登录"的那条判断，`load` 与旧文件迁移都走它：坏文本不能炸。
        assert!(Auth::from_json("not json").is_none());
        assert!(Auth::from_json("").is_none());
        // 四条 cookie 是**必填**字段（没有 `#[serde(default)]`，只有 refresh_token/saved_at_unix 有）：
        // 少给任何一条就是解析失败，而解析失败一律当"没登录"。这是故意的——
        // 拿一份缺 buvid3 的半成品去构造 `Account`，只会在第一个接口上撞 `-101`，
        // 症状比"重新扫一次码"难查得多。（第三十四轮就是这条把我自己的假设打回来了。）
        assert!(Auth::from_json(r#"{"dede_user_id":"1","sessdata":"2"}"#).is_none());
        // 四条都在但值是空串：能解析，判不合格是 `is_complete` 的活（迁移那一路用得到）。
        let blank = Auth::from_json(
            r#"{"dede_user_id":"","sessdata":"","bili_jct":"","buvid3":""}"#,
        )
        .expect("四条键都在就应当能解析");
        assert!(!blank.is_complete());
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
mod config_tests {
    use crate::api::stream_select::DEFAULT_QUALITY_CAP;
    use crate::store::config::{
        clamp_quality, quality_name, CodecPreference, Config, MAX_QUALITY_ID, MIN_QUALITY_ID,
        QUALITY_CHOICES, DEFAULT_QUALITY_ID,
    };

    #[test]
    fn quality_cap_is_clamped_into_the_supported_range() {
        // P13 起上限不再是 116 那条硬红线，而是"接口里最高的档"（127=8K）。
        // 0 仍然要收到下界：起播请求一个不存在的档会让整条路没法走。
        assert_eq!(clamp_quality(0), MIN_QUALITY_ID);
        assert_eq!(clamp_quality(6), MIN_QUALITY_ID);
        assert_eq!(clamp_quality(116), 116);
        // 4K 现在是**允许**的（他要能在设置里选到），所以不再被收回。
        assert_eq!(clamp_quality(120), 120);
        assert_eq!(clamp_quality(125), 125);
        assert_eq!(clamp_quality(9999), MAX_QUALITY_ID);
        assert_eq!(clamp_quality(MAX_QUALITY_ID), MAX_QUALITY_ID);
    }

    #[test]
    fn default_is_the_device_friendly_cap() {
        // 默认仍是 116：目标机（i5-6200U）能硬解到的最高档。
        // 这条把"上限放宽"与"默认放宽"分开——前者是给他选的，后者是我们替他选的。
        assert_eq!(DEFAULT_QUALITY_ID, DEFAULT_QUALITY_CAP);
        let empty = Config::from_json("{}").expect("空对象应当能读");
        assert_eq!(empty.max_quality, DEFAULT_QUALITY_ID);
        assert_eq!(empty.codec, CodecPreference::Hevc);
        assert!(empty.mpv_path.is_empty(), "空 = 自动找 PATH/运行目录");
        assert!(!empty.debug_log, "调试模式默认关（不产生 debug.log）");
    }

    #[test]
    fn quality_choices_are_the_tiers_bilibili_actually_has() {
        // 他列了"1080P、1080P 60帧、2K、4K"，但 B 站的 qn 里**没有 2K** 这一档，
        // 所以设置页只给这三个真值。这里钉住"不造名字"这条口径。
        assert!(
            !QUALITY_CHOICES.iter().any(|(_, label)| label.contains("2K")),
            "B 站没有 2K 档，界面上不许出现假档位"
        );
        assert_eq!(QUALITY_CHOICES.len(), 3);
        for (qn, _) in QUALITY_CHOICES {
            assert!(!quality_name(qn).contains("未知"), "档位 {qn} 应当有名字");
        }
    }

    #[test]
    fn codec_preference_serializes_lowercase() {
        // 落盘格式定死：手改文件的人写 "hevc"/"avc"/"av1" 就该读得出来。
        let written = serde_json::to_string(&CodecPreference::Av1).expect("序列化应当成功");
        assert_eq!(written, "\"av1\"");
        let parsed: CodecPreference = serde_json::from_str("\"avc\"").expect("小写应当能读");
        assert_eq!(parsed, CodecPreference::Avc);
        // 认不出的写法不许静默变成默认值：这条要它读**失败**
        // （字段整个缺失才走 `#[serde(default)]`，写错值属于用户能看见的问题）。
        assert!(serde_json::from_str::<CodecPreference>("\"h266\"").is_err());
    }

    #[test]
    fn written_config_roundtrips_all_four_settings() {
        let config = Config {
            max_quality: 120,
            codec: CodecPreference::Av1,
            mpv_path: "D:\\tools\\mpv.exe".to_string(),
            debug_log: true,
        };
        let json = serde_json::to_string(&config).expect("序列化应当成功");
        let back = Config::from_json(&json).expect("往返应当能读");
        assert_eq!(back, config);
    }

    #[test]
    fn broken_config_text_is_not_a_panic() {
        // 读不到就用默认值（`load_or_create` 会顺手写一份回去并强制开日志），
        // 这里只验"不炸、给 None"。
        assert!(Config::from_json("not json").is_none());
        assert!(Config::from_json("").is_none());
    }

    #[test]
    fn quality_names_cover_the_tiers_we_allow() {
        // 设置页显示的是这一串，所以"认不出就不编名字"要钉住。
        assert_eq!(quality_name(DEFAULT_QUALITY_CAP), "1080P 60帧（qn=116）");
        assert_eq!(quality_name(64), "720P（qn=64）");
        assert_eq!(quality_name(120), "4K 超清（qn=120）");
        assert_eq!(quality_name(127), "8K 超高清（qn=127）");
        assert!(quality_name(99).contains("qn=99"));
    }
}

#[cfg(test)]
mod login_tests {
    use crate::api::login::{QrPollState, auth_from_cookies, needs_buvid3_backfill};
    use crate::store::auth::Auth;

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
        let credential = auth_from_cookies(&full_cookies(), "refresh")
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
        assert!(auth_from_cookies(&partial, "refresh").is_none());
    }

    #[test]
    fn saved_credential_roundtrips_after_login() {
        let credential = auth_from_cookies(&full_cookies(), "refresh").unwrap();
        let json = serde_json::to_string(&credential).unwrap();
        let back: Auth = serde_json::from_str(&json).unwrap();
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
mod profile_tests {
    use crate::api::{format_seconds, progress_text};
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
    fn progress_text_hides_zero_and_unknown() {
        // 历史与待看共用这一份口径：0 是"没看过"、-1 是 B 站偶尔给的"未知"，
        // 两种都不许显示成"看到 00:00"（那是"看了但一秒没看"的误导）。
        assert_eq!(progress_text(0), "");
        assert_eq!(progress_text(-1), "");
        assert_eq!(progress_text(45), "看到 00:45");
        assert_eq!(progress_text(3725), "看到 1:02:05");
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
mod touch_scroll_tests {
    use crate::ui::scroll::target_offset;
    use gpui_kit::{Pixels, px};

    // 约定与 gpui 一致：`offset` 是负值（0 = 顶），`max` 是正的可用行程，`dy` 是手指位移
    // （往下为正）。方向反了不会编译失败，只会在真机上"手指往上、内容往下"，所以钉死。
    #[test]
    fn dragging_down_reveals_content_above_and_stops_at_the_top() {
        let zero: Pixels = px(0.);
        // 已经停在顶部还继续往下拖：留在 0，不许拖出正偏移（那会露出内容上方的一片空白）。
        assert_eq!(target_offset(zero, px(50.), px(500.)), zero);
        // 从中段往下拖 30：往"顶"的方向回 30，但不越过 0。
        assert_eq!(target_offset(px(-10.), px(30.), px(500.)), zero);
        assert_eq!(target_offset(px(-100.), px(30.), px(500.)), px(-70.));
    }

    #[test]
    fn dragging_up_scrolls_towards_the_end_and_stops_at_the_bottom() {
        // 手指往上走 = 往下看：偏移跟着变负。
        assert_eq!(target_offset(px(-50.), px(-10.), px(100.)), px(-60.));
        // 已经到底还继续往上拖：停在 `0 - max`，攒不出"要往回走一大段才动"的死区。
        assert_eq!(target_offset(px(-100.), px(-50.), px(100.)), px(-100.));
    }

    #[test]
    fn no_movement_keeps_the_offset() {
        assert_eq!(target_offset(px(-42.), px(0.), px(100.)), px(-42.));
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
    use crate::store::auth::Auth;

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
        let options = quality_options(&formats, &[], &[], 116);
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].label, "720P 准高清");
    }

    /// 封顶档以外的一律不给按钮（P11 修的缺陷）。
    ///
    /// 以前这里给的是 B 站 `support_formats` 的**全集**：界面能点出 4K/HDR/杜比，
    /// 点下去 `pick_video` 就按那个档选流——我们自己写的 1080P 上限被自己的按钮顶穿，
    /// 而"高画质下载"下的也会是超限的那一路。
    #[test]
    fn quality_options_hide_the_tiers_over_our_cap() {
        let formats = vec![
            support(126, " Dolby", "杜比视界"),
            support(120, "4K 超清", "4K"),
            support(116, "1080P60 高帧率", "1080P60"),
            support(80, "1080P 高清", "1080P"),
            support(32, "480P", "480P"),
        ];
        let options = quality_options(&formats, &[], &[], 116);
        let qn: Vec<u64> = options.iter().map(|option| option.qn).collect();
        assert_eq!(qn, vec![116, 80, 32]);

        // 封顶更小时跟着再收一档，顺序仍是 B 站给的顺序。
        let options = quality_options(&formats, &[], &[], 64);
        let qn: Vec<u64> = options.iter().map(|option| option.qn).collect();
        assert_eq!(qn, vec![32]);

        // 全是超限档时退回原样：宁可给一个超限的选项，也不要一排点不动的按钮。
        let only_ultra = vec![support(120, "4K 超清", "4K")];
        let options = quality_options(&only_ultra, &[], &[], 116);
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].qn, 120);
    }

    #[test]
    fn cookie_header_lists_only_what_we_have() {
        let credential = Auth {
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

        let partial = Auth {
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
mod home_tests {
    use crate::api::home::{HOME_PAGE_SIZE, is_archive_goto, play_text_of};
    use bpi_rs::video::recommend::HomeRmdStat;

    #[test]
    fn only_archive_goto_can_be_opened() {
        // 推荐流里混着直播与剧集项（`ref/bpi-rs/src/video/recommend.rs:135-136`），
        // 它们的 bvid 不是稿件号，放进可点卡片里就是"点了进详情页然后报错"。
        assert!(is_archive_goto("av"));
        assert!(!is_archive_goto("live"));
        assert!(!is_archive_goto("ogv"));
        assert!(!is_archive_goto(""));
    }

    #[test]
    fn missing_stat_shows_nothing_instead_of_zero() {
        // `stat` 在这个接口里是 Option。写 0 会把"没给数据"说成"没人看"。
        assert_eq!(play_text_of(None), "");
        let stat = HomeRmdStat {
            view: 15_000,
            danmaku: 20,
            like: 30,
        };
        assert_eq!(play_text_of(Some(&stat)), "1.5万");
    }

    #[test]
    fn page_size_stays_inside_the_api_bounds() {
        // bpi-rs 直接拒 0 与 >30（`ref/bpi-rs/src/video/params.rs:366-371`），
        // 这个常量要是被改成越界，第一次请求就会在本地失败而不是网络失败——更难查。
        assert!(HOME_PAGE_SIZE > 0 && HOME_PAGE_SIZE <= 30);
    }
}

#[cfg(test)]
mod comment_tests {
    use crate::api::comments::{
        COMMENT_PAGE_SIZE, CommentPage, CommentSortKind, relative_time,
    };

    fn page(items: usize, page_no: u32, total: Option<u64>) -> CommentPage {
        CommentPage {
            items: (0..items).map(|_| Default::default()).collect(),
            total,
            page: page_no,
            sort: CommentSortKind::Hot,
        }
    }

    #[test]
    fn video_comment_target_type_is_one() {
        // 网上不少帖子写"视频=11"，但 11 是相簿；bpi-rs 的类型与它的 live 测试都是 1。
        // 写错这个数字的症状是"评论永远空"，所以钉成断言。
        assert_eq!(crate::api::comments::COMMENT_TYPE_VIDEO, 1);
    }

    #[test]
    fn has_more_uses_total_when_available() {
        // 第 1 页 20 条、总共 20 条 → 到底了。
        assert!(!page(20, 1, Some(20)).has_more());
        assert!(page(20, 1, Some(21)).has_more());
        // 第 2 页只回 5 条、总共 25 条 → 正好读完了。
        assert!(!page(5, 2, Some(25)).has_more());
        assert!(page(5, 2, Some(26)).has_more());
    }

    #[test]
    fn has_more_falls_back_to_full_page_without_total() {
        // `page` 那一层是 Option：没给总数时只能按"这一页取满了 ps"推断还有下一页。
        assert!(page(COMMENT_PAGE_SIZE as usize, 1, None).has_more());
        assert!(!page(3, 1, None).has_more());
    }

    #[test]
    fn relative_time_steps_down_from_days() {
        let now = 1_700_000_000;
        assert_eq!(relative_time(now, now), "刚刚");
        assert_eq!(relative_time(now - 59, now), "刚刚");
        assert_eq!(relative_time(now - 60, now), "1分钟前");
        assert_eq!(relative_time(now - 3_600, now), "1小时前");
        assert_eq!(relative_time(now - 86_400, now), "1天前");
        assert_eq!(relative_time(now - 30 * 86_400, now), "1个月前");
        assert_eq!(relative_time(now - 365 * 86_400, now), "1年前");
        // 评论时间戳在未来（时钟漂移）不该显示出负数。
        assert_eq!(relative_time(now + 600, now), "刚刚");
    }

    #[test]
    fn sort_labels_are_chinese() {
        assert_eq!(CommentSortKind::Hot.label(), "最热");
        assert_eq!(CommentSortKind::Latest.label(), "最新");
    }
}

#[cfg(test)]
mod favorites_tests {
    use crate::api::favorites::pick_bvid;
    use crate::pages::matches_keyword;

    #[test]
    fn bvid_prefers_the_new_key_and_falls_back_to_the_old_one() {
        // B 站这个字段历史上换过名，库里两个键都留着（`ref/bpi-rs/src/fav/list.rs:80-81`）。
        assert_eq!(pick_bvid(Some("BV1new".into()), Some("BVold".into())), "BV1new");
        assert_eq!(pick_bvid(None, Some("BVold".into())), "BVold");
        // 空串也算没有：拿不到稿件号的行进不了详情页，必须被跳过而不是画成一张点不动的卡。
        assert_eq!(pick_bvid(Some("".into()), Some("BVold".into())), "BVold");
        assert!(pick_bvid(Some("".into()), None).is_empty());
        assert!(pick_bvid(None, None).is_empty());
    }

    #[test]
    fn keyword_filter_matches_title_or_author_and_lets_blank_through() {
        // 空关键词一律放行：「还没输入」与「输入了又删干净」是同一种状态，不许出现空列表。
        assert!(matches_keyword("任何标题", "任何UP", ""));
        assert!(matches_keyword("任何标题", "任何UP", "   "));
        assert!(matches_keyword("Rust 入门教程", "UP主", "rust")); // 拉丁字母忽略大小写
        assert!(matches_keyword("某视频", "老番茄", "番茄")); // 中文按字面匹配
        assert!(!matches_keyword("某视频", "老番茄", "不存在的词"));
    }
}

#[cfg(test)]
mod space_tests {
    use crate::api::space::{SpaceSection, VideoOrder, parse_tlist};
    use serde_json::json;

    #[test]
    fn tlist_becomes_chips_sorted_by_count() {
        let value = json!({
            "17": {"tid": 17, "count": 3, "name": "单机游戏"},
            "28": {"tid": 28, "count": 12, "name": "游戏"}
        });
        let filters = parse_tlist(&value);
        assert_eq!(filters.len(), 2);
        // 按投稿数从多到少：serde_json 的对象是无序的，不排序的话 chips 每次刷新都会换位置。
        assert_eq!(filters[0].name, "游戏");
        assert_eq!(filters[1].name, "单机游戏");
        assert_eq!(filters[0].tid, 28);
    }

    #[test]
    fn malformed_tlist_gives_no_chips_instead_of_failing() {
        // 这一格在 bpi-rs 里就是 `serde_json::Value`（没有类型可跟）。
        // 形状变了只允许"少一排 chips"，不许把整个投稿块判成失败。
        assert!(parse_tlist(&json!(null)).is_empty());
        assert!(parse_tlist(&json!([])).is_empty());
        assert!(parse_tlist(&json!({"17": "游戏"})).is_empty());
        assert!(parse_tlist(&json!({"17": {"count": 2}})).is_empty());
        assert!(parse_tlist(&json!({"17": {"name": "", "count": 2}})).is_empty());
    }

    #[test]
    fn tid_falls_back_to_the_map_key() {
        // B 站两处都给 tid；只给键的时候也不该丢这一格（否则用户少一个筛选项）。
        let filters = parse_tlist(&json!({"17": {"name": "单机游戏", "count": 1}}));
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].tid, 17);
        assert_eq!(filters[0].count, 1);
    }

    #[test]
    fn cover_ratio_uses_the_response_and_falls_back_to_widescreen() {
        // 图文的图有横有竖：接口给了 width/height 就必须按它摆，否则竖图会被裁掉半张脸。
        assert!((crate::api::space::cover_ratio(300, 400) - 0.75).abs() < 1e-6);
        // 缺数据（0）时退回 16:9，不能算出 0 或 NaN 把整档高度塌成一条缝。
        assert!(crate::api::space::cover_ratio(0, 0) > 1.7 && crate::api::space::cover_ratio(0, 0) < 1.8);
        assert!(crate::api::space::cover_ratio(640, 0) > 1.7);
    }

    #[test]
    fn last_page_rounds_up_without_growing_an_extra_page() {
        // 这里踩过一次：写成 `total / ps + 1` 在"正好 30 个"时会多给一页，
        // 用户点"下一页"看到一片空。整除的边界必须算准。
        assert_eq!(crate::api::space::last_page(0, 30), 1);
        assert_eq!(crate::api::space::last_page(1, 30), 1);
        assert_eq!(crate::api::space::last_page(30, 30), 1);
        assert_eq!(crate::api::space::last_page(31, 30), 2);
        assert_eq!(crate::api::space::last_page(60, 30), 2);
        // 每页大小给 0（接口没回 page_size）时不能除崩，也不能算出 0 页。
        assert_eq!(crate::api::space::last_page(100, 0), 1);
    }

    #[test]
    fn section_and_order_labels_read_in_chinese() {
        assert_eq!(SpaceSection::Videos.label(), "投稿");
        assert_eq!(SpaceSection::Posts.label(), "图文");
        assert_eq!(SpaceSection::Folders.label(), "收藏");
        assert_eq!(VideoOrder::Latest.label(), "最新发布");
        assert_eq!(VideoOrder::MostPlay.label(), "最多播放");
        assert_eq!(VideoOrder::MostFavorite.label(), "最多人收藏");
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

#[cfg(test)]
mod download_tests {
    use std::path::Path;

    use crate::api::download::{
        self, format_bytes, numbered_stem, percent, sanitize_file_stem,
    };
    use crate::api::stream::{QualityOption, StreamChoice};

    /// 造一份"选定了一条流"的结果。`is_dash=false` 就是 durl 回退那一路。
    fn choice(is_dash: bool) -> StreamChoice {
        StreamChoice {
            cid: 777,
            video_url: "https://v.example/video.m4s".to_string(),
            audio_url: is_dash.then(|| "https://v.example/audio.m4s".to_string()),
            backup_urls: vec!["https://b.example/video.m4s".to_string()],
            audio_backup_urls: if is_dash {
                vec!["https://b.example/audio.m4s".to_string()]
            } else {
                Vec::new()
            },
            video_size: if is_dash { Some(100) } else { None },
            audio_size: if is_dash { Some(30) } else { None },
            video_ext: if is_dash { "m4v" } else { "mp4" },
            audio_ext: "m4a",
            quality: 80,
            quality_label: "1080P 高清".to_string(),
            codecs: "hev1.1.6.L150.90".to_string(),
            available: vec![QualityOption {
                qn: 80,
                label: "1080P 高清".to_string(),
            }],
            is_dash,
            timelength_ms: 60_000,
        }
    }

    #[test]
    fn illegal_characters_become_spaces_and_blanks_collapse() {
        // Windows 禁的九个字符全命中：一律换空格，再把连续空白折成一个。
        assert_eq!(
            sanitize_file_stem("预告:第3集 *?<>|\"/\\ 收尾"),
            "预告 第3集 收尾"
        );
        // 换行与控制字符也是空格（标题里真的会带 \n）。
        assert_eq!(sanitize_file_stem("标题\n  第二行"), "标题 第二行");
        // 结尾的空格与点都要去掉：Windows 不允许文件名以它们收尾。
        assert_eq!(sanitize_file_stem("  名字...  "), "名字");
        // 全是非法字符 → 空串，由调用方（file_stem）退回稿件号。
        assert_eq!(sanitize_file_stem("***"), "");
        assert_eq!(download::file_stem("***", "BV1xx", None), "BV1xx");
    }

    #[test]
    fn stem_is_counted_in_characters_not_bytes() {
        // 60 个中文字符：按字节截会切在多字节中间，按字符截不会。
        let long = "测".repeat(80);
        let stem = sanitize_file_stem(&long);
        assert_eq!(stem.chars().count(), 60);
        assert!(stem.chars().all(|ch| ch == '测'));
    }

    #[test]
    fn multi_part_titles_carry_the_page_number() {
        let stem = download::file_stem("总集篇", "BV1yy", Some((3, "下集")));
        assert_eq!(stem, "总集篇 P3 下集");
        // 单 P 的稿件不加多余的字。
        assert_eq!(download::file_stem("一支pv", "BV1zz", None), "一支pv");
    }

    #[test]
    fn second_copy_gets_a_numbered_suffix() {
        assert_eq!(numbered_stem("名字", 0), "名字");
        assert_eq!(numbered_stem("名字", 1), "名字 (2)");
        assert_eq!(numbered_stem("名字", 4), "名字 (5)");
    }

    #[test]
    fn byte_readings_are_human_sized() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(100 * 1024 * 1024), "100.0 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024 + 512 * 1024 * 1024), "3.50 GB");
    }

    #[test]
    fn percent_refuses_to_claim_done() {
        assert_eq!(percent(50, Some(100)), Some(50.));
        // 没有分母就不给百分比（界面改用不定进度，不猜总数）。
        assert_eq!(percent(50, None), None);
        assert_eq!(percent(50, Some(0)), None);
        // 已经写得比分母还多（B 站的 size 偶尔偏小）也不能显示 100%：
        // "100%" 在这套界面里只属于 Finished。
        let value = percent(1_000, Some(100)).unwrap_or_default();
        assert!(value <= 99.9, "超量时应当夹住，实际 {value}");
    }

    #[test]
    fn request_only_counts_total_when_every_track_is_known() {
        let request = download::request_for(&choice(true), "标题 · 1080P".to_string(), "标题".to_string())
            .expect("DASH 计划应当能拼出下载请求");
        assert_eq!(request.total_bytes(), Some(130));
        assert_eq!(request.extensions(), vec!["m4v", "m4a"]);
        let targets = request.targets(Path::new("D:\\片"));
        assert_eq!(targets.len(), 2);
        assert_eq!(
            targets[0].file_name().map(|name| name.to_string_lossy().to_string()),
            Some("标题.m4v".to_string())
        );
        assert_eq!(
            targets[1].file_name().map(|name| name.to_string_lossy().to_string()),
            Some("标题.m4a".to_string())
        );

        // 只给了一条轨（音频接口没给流）：总数就是视频那一轨，且不该有第二个文件。
        let mut only_video = choice(true);
        only_video.audio_url = None;
        let request = download::request_for(&only_video, "标题".to_string(), "标题".to_string())
            .expect("单轨计划应当能拼出下载请求");
        assert_eq!(request.total_bytes(), Some(100));
        assert_eq!(request.extensions(), vec!["m4v"]);
        assert_eq!(request.targets(Path::new(".")).len(), 1);

        // 音频有轨但大小缺失：这时候**不给**总数，别画一个假的分母。
        let mut no_audio_size = choice(true);
        no_audio_size.audio_size = None;
        let request = download::request_for(&no_audio_size, "标题".to_string(), "标题".to_string())
            .expect("计划应当能拼出下载请求");
        assert_eq!(request.total_bytes(), None);
    }

    /// durl 是多段文件，要按 `order` 拼接才是完整一片。这一版不实现拼接，
    /// 所以这里必须**明确拒绝**，而不是"看起来能下、下出来只有第一段"。
    #[test]
    fn single_file_direct_links_are_refused_with_a_reason() {
        let message = download::request_for(&choice(false), "标题".to_string(), "标题".to_string())
            .expect_err("durl 计划不该被接受");
        assert!(message.contains("分段直链"), "要说清楚拒的是什么：{message}");
    }

    #[test]
    fn with_stem_renames_both_targets() {
        let request = download::request_for(&choice(true), "标题".to_string(), "标题".to_string())
            .expect("计划应当能拼出下载请求")
            .with_stem("标题 (2)".to_string());
        let names: Vec<String> = request
            .targets(Path::new("."))
            .iter()
            .filter_map(|path| path.file_name().map(|name| name.to_string_lossy().to_string()))
            .collect();
        assert_eq!(names, vec!["标题 (2).m4v", "标题 (2).m4a"]);
    }
}

#[cfg(test)]
mod collection_tests {
    use crate::api::collection::{CollectionArchives, CollectionEpisode, date_text};

    fn episode(bvid: &str) -> CollectionEpisode {
        CollectionEpisode {
            bvid: bvid.to_string(),
            title: bvid.to_string(),
            duration_text: "10:00".to_string(),
            play_text: "1 播放".to_string(),
            date_text: String::new(),
        }
    }

    #[test]
    fn date_text_uses_beijing_time_and_refuses_zero() {
        // 0 一律空串：缺日期不编一个 1970-01-01 出来。
        assert_eq!(date_text(0), "");
        // 2024-01-01 00:00 UTC = 北京时间 08:00，两种算法都是 1 月 1 日，不暴露差异。
        assert_eq!(date_text(1_704_067_200), "2024-01-01");
        // 2023-12-31 22:00 UTC = 北京时间 2024-01-01 06:00 —— 这一格才是"按北京时间"的证据：
        // 用 UTC 会显示成 2023-12-31，凌晨发布的稿件会集体少一天。
        assert_eq!(date_text(1_704_060_000), "2024-01-01");
        // 闰年边界：2024-02-29 存在，换算不能跳到 3 月。
        assert_eq!(date_text(1_709_164_800), "2024-02-29");
        // 跨年：北京时间 2023-01-01 00:30 发布（= UTC 2022-12-31 16:30）。
        // 按 UTC 算会显示成 2022-12-31——"元旦零点的稿件"会集体归到去年。
        assert_eq!(date_text(1_672_504_200), "2023-01-01");
    }

    #[test]
    fn has_more_prefers_the_authority_it_was_given() {
        let mut page = CollectionArchives {
            title: "合集".to_string(),
            desc: String::new(),
            total: Some(100),
            page: 1,
            items: (0..30).map(|index| episode(&format!("BV{index:06}"))).collect(),
        };
        // 有 `total`：按已加载条数比总数。
        assert!(page.has_more(30));
        assert!(!page.has_more(100));

        // 接口没给总数（`None`）：只能退回"这一页给满了就还能翻"。
        page.total = None;
        assert!(page.has_more(30));
        page.items = vec![episode("BV1")];
        assert!(!page.has_more(31));
        // 空列表且没总数：不翻（否则"再加载一页"会永远点得动）。
        page.items.clear();
        assert!(!page.has_more(0));

        // 总数比已加载还小（`page.total` 是"页数"而非"条目数"时的样子）：那个数一律当没给，
        // 于是退回"本页给满了就还能翻"——宁可多点一次拿到空页，
        // 也绝不显示"共 7 集 · 已加载 30"这种自相矛盾的一行。
        page.total = Some(7);
        page.items = (0..30).map(|index| episode(&format!("BV{index:06}"))).collect();
        assert_eq!(page.total_known(30), None);
        assert!(page.has_more(30));
    }
}

#[cfg(test)]
mod season_tests {
    use crate::api::detail::parse_ugc_season;
    use serde_json::{Value, json};

    fn episode(bvid: &str, title: &str, videos: u64, duration: u64) -> Value {
        json!({
            "bvid": bvid,
            "cid": 1,
            "title": title,
            "arc": { "title": title, "duration": duration, "videos": videos },
            "pages": [],
        })
    }

    fn payload(sections: Value) -> Value {
        json!({
            "aid": 1,
            "ugc_season": {
                "id": 4294056,
                "title": "物理er的本科专业合集",
                "ep_count": 189,
                "sections": sections,
            }
        })
    }

    #[test]
    fn a_video_outside_a_season_has_no_panel() {
        // 没有 `ugc_season`：正常情况（单发稿件），返回 None 而不是"空的一节"。
        assert!(parse_ugc_season(&json!({ "aid": 1 }), "BV1").is_none());
        // 有这块但不是对象（接口偶尔给 null）：同样 None。
        assert!(parse_ugc_season(&json!({ "ugc_season": Value::Null }), "BV1").is_none());
        // 有合集却一集都列不出来：也不摆空节。
        let brief = parse_ugc_season(&payload(json!([])), "BV1");
        assert!(brief.is_none(), "sections 为空应当是 None，拿到 {brief:?}");
    }

    #[test]
    fn flat_index_counts_across_sections_and_marks_current() {
        let data = payload(json!([
            { "id": 1, "title": "电动/猞狸", "episodes": [episode("BV1a", "第一集", 1, 60), episode("BV1b", "第二集", 4, 90)] },
            { "id": 2, "title": "量力/度规", "episodes": [episode("BV1c", "第三集", 1, 120)] },
        ]));
        let brief = parse_ugc_season(&data, "BV1c").expect("这份载荷应当能解析");
        assert_eq!(brief.listed, 3);
        // 行号跨分段连续：第二个分段的第一集是"第 3 集"，不是"第 1 集"。
        assert_eq!(brief.current_index, 3);
        // 多分段才给 chips。
        assert!(brief.has_sections());
        // `arc.videos > 1` 就是"分集自己还有分 P"那种复杂情况，必须显出来。
        assert_eq!(brief.sections[0].episodes[1].videos, 4);
        assert_eq!(brief.sections[0].episodes[1].duration_text, "01:30");
        assert_eq!(brief.ep_count, 189);
        // 接口声明 189 集却只给了 3 集：这个差额界面要说，不藏。
        assert_ne!(brief.ep_count as usize, brief.listed);
    }

    #[test]
    fn single_section_seasons_need_no_section_chips() {
        let data = payload(json!([
            { "id": 1, "title": "全一辑", "episodes": [episode("BV1a", "只此一集", 1, 45)] },
        ]));
        let brief = parse_ugc_season(&data, "BV1a").expect("单分段合集应当能解析");
        assert!(!brief.has_sections());
        assert_eq!(brief.current_index, 1);
    }

    #[test]
    fn missing_pieces_degrade_one_cell_at_a_time() {
        // 缺 `arc`：标题退回 `episodes[].title`，时长退空、分 P 数按 1（不编）。
        let data = payload(json!([
            { "id": 1, "episodes": [
                { "bvid": "BV1x", "title": "只有标题的一条" },
                { "arc": { "title": "只有 arc 标题的一条", "duration": 30 }, "bvid": "BV1y" },
                { "title": "没有 bvid，点了也打不开" },
            ] },
        ]));
        let brief = parse_ugc_season(&data, "BV1y").expect("缺字段不该让整块失败");
        assert_eq!(brief.listed, 2, "没有 bvid 的那条应当被跳过");
        assert_eq!(brief.skipped, 1, "跳过的条数要能说出来");
        assert_eq!(brief.sections[0].episodes[0].title, "只有标题的一条");
        assert_eq!(brief.sections[0].episodes[0].duration_text, "");
        assert_eq!(brief.sections[0].episodes[0].videos, 1);
        // 标题优先 `episodes[].title`，它是空的才退回 `arc.title`。
        assert_eq!(brief.sections[0].episodes[1].title, "只有 arc 标题的一条");
        assert_eq!(brief.sections[0].episodes[1].duration_text, "00:30");
        // 当前集没在列出的名单里（例如接口只给了前 100 集）：序号给 0，界面那句"当前这一集不在名单里"才成立。
        assert_eq!(brief.current_index, 2);
        let absent = parse_ugc_season(&data, "BV1不在这里").expect("同上");
        assert_eq!(absent.current_index, 0);
    }

    #[test]
    fn wrong_typed_numbers_are_treated_as_absent() {
        // `videos` 给成字符串、`ep_count` 给成小数：一律当"没有"，不猜、也不 panic。
        let data = json!({
            "ugc_season": {
                "id": "4294056",
                "ep_count": 1.5,
                "sections": [{ "episodes": [{ "bvid": "BV1z", "arc": { "videos": "4" } }] }],
            }
        });
        let brief = parse_ugc_season(&data, "BV1z").expect("这一份还能列出一集，就该给出面板");
        assert_eq!(brief.season_id, 0, "id 是字符串时当没有");
        assert_eq!(brief.ep_count, 0, "总数给成小数时当没有，界面那行就不说总数");
        assert_eq!(brief.sections[0].episodes[0].videos, 1, "分 P 数给成字符串时按单 P");
    }
}

#[cfg(test)]
mod probe_tests {
    use crate::api::probe::{
        choose, describe, host_of, is_playable_status, ordered_urls, ProbeOutcome,
    };

    fn ok(url: &str, status: u16) -> (String, Result<u16, String>) {
        (url.to_string(), Ok(status))
    }

    fn bad(url: &str, reason: &str) -> (String, Result<u16, String>) {
        (url.to_string(), Err(reason.to_string()))
    }

    #[test]
    fn candidates_put_the_primary_first_without_duplicates() {
        // 主地址永远排第一；B 站偶尔把同一个域名同时写进 baseUrl 与 backupUrl，
        // 空串也真的出现过（给空数组以外的空字符串），照原样探就是白跑一趟。
        let backups = vec![
            "https://a/x.m4s".to_string(),
            String::new(),
            "https://p/x.m4s".to_string(),
        ];
        let urls = ordered_urls("https://p/x.m4s", &backups);
        assert_eq!(
            urls,
            vec!["https://p/x.m4s".to_string(), "https://a/x.m4s".to_string()]
        );
        // 没有备用域名时就只有主地址一条。
        assert_eq!(ordered_urls("https://p/x.m4s", &[]), vec!["https://p/x.m4s"]);
        // 主地址是空串（理论上不该发生，但接口给过空字段）：不占位，备用域名照常排。
        assert_eq!(ordered_urls("", &backups).len(), 2, "{:?}", ordered_urls("", &backups));
    }

    #[test]
    fn playable_statuses_are_two_and_three_hundred_range() {
        assert!(is_playable_status(200));
        // 有些 CDN 对 `Range: bytes=0-0` 直接回 200 而不是 206，那也是通。
        assert!(is_playable_status(206));
        assert!(is_playable_status(302));
        // 403 是凭据/权限问题，416 是 Range 不被接受，5xx 是节点抽风：都不算能播。
        for status in [400u16, 401, 403, 404, 416, 500, 503] {
            assert!(!is_playable_status(status), "{status} 不该算通");
        }
    }

    #[test]
    fn choose_stops_at_the_first_playable_candidate() {
        let results = vec![
            ok("https://a/x.m4s", 403),
            ok("https://b/x.m4s", 206),
            ok("https://c/x.m4s", 200),
        ];
        let outcome = choose(&results);
        assert_eq!(outcome.url.as_deref(), Some("https://b/x.m4s"));
        // 探到能用就停：第三条根本没问，所以 `attempted` 是"用掉第几个"。
        assert_eq!(outcome.attempted, 2);
        assert_eq!(outcome.failures.len(), 1);
        assert!(outcome.fell_back(), "换成第二个域名应当算换过");
        // 那行话里带的是**主机名**（不是完整直链，里面有签名令牌）与状态码。
        assert_eq!(outcome.failures[0], "a：HTTP 403", "{}", outcome.failures[0]);
    }

    #[test]
    fn choose_reports_no_fallback_when_the_primary_works() {
        let outcome = choose(&[ok("https://a/x.m4s", 200)]);
        assert_eq!(outcome.attempted, 1);
        assert!(!outcome.fell_back());
        assert!(describe("视频", &outcome).is_empty(), "主地址就通就不该多打一行字");
    }

    #[test]
    fn choose_keeps_trying_the_primary_when_every_candidate_fails() {
        // 全不通 → `url` 是 None：调用方仍要把**主地址**交给 mpv（探测本身可能误判，
        // 而 mpv 自己还有一次机会）。界面那行要说清"仍按主地址试一次"。
        let results = vec![bad("https://a/x.m4s", "请求失败: timeout"), ok("https://b/x.m4s", 403)];
        let outcome = choose(&results);
        assert_eq!(outcome.url, None);
        assert_eq!(outcome.attempted, 2);
        let text = describe("视频", &outcome);
        assert!(text.contains("仍按主地址"), "{text}");
        assert!(text.contains("timeout"), "{text}");
    }

    #[test]
    fn untouched_outcome_adds_no_noise() {
        // durl 回退且没给备用域名时候选是空的：这时不该在界面上写"探测失败"。
        let outcome = ProbeOutcome::untouched();
        assert_eq!(outcome.attempted, 0);
        assert!(describe("音频", &outcome).is_empty());
        assert!(!outcome.fell_back());
    }

    #[test]
    fn host_extraction_never_shows_the_signed_url() {
        // 界面那行只写主机名：完整直链里是签名令牌（deadline/mid/gen），不该显示出来。
        assert_eq!(
            host_of("https://cn.example.cdn/bili/1.m4s?deadline=1&gen=abc"),
            "cn.example.cdn"
        );
        assert_eq!(host_of("not a url"), "not a url", "认不出 scheme 就原样给出，不编一个空的");
        assert_eq!(host_of("https://only.host"), "only.host");
    }
}

#[cfg(test)]
mod auto_page_tests {
    use crate::ui::scroll::{auto_page_wanted, is_at_bottom, AUTO_PAGE_MAX_ITEMS};

    // `offset` 是滚动句柄给的**负**偏移（0 = 顶部），`max` 是正的可用行程，两者符号不一样，
    // 所以判"到底"要拿 `max - |offset|` 与提前量比。方向写反的症状是"停在半路就翻页"，
    // 在真机上很容易被误认成"接口又给了一页重复内容"，所以这里钉住。
    #[test]
    fn bottom_detection_uses_absolute_offset() {
        // 到底：行程 1000、已经滚了 1000。
        assert!(is_at_bottom(-1000., 1000., 240.));
        // 差 200 还没到底，但落在 240 的提前量里 → 触发（把等待藏在滚动里）。
        assert!(is_at_bottom(-800., 1000., 240.));
        // 离底还远：不许触发。
        assert!(!is_at_bottom(-300., 1000., 240.));
        // 顶部（offset=0）在长列表里当然不算到底。
        assert!(!is_at_bottom(0., 1000., 240.));
    }

    #[test]
    fn zero_travel_counts_as_bottom_by_definition() {
        // 内容比视口短 = 零行程，按定义就是"在底部"。
        // 这条**不是**"所以会自动翻页"：那一层由 `should_auto_page` 的 `scrollable()` 挡掉，
        // 这里只是把纯函数的边界写清楚，免得以后有人以为它返回 false。
        assert!(is_at_bottom(0., 0., 240.));
        // 拖过头（偏移比行程还大）仍然算在底部，不会翻不回来。
        assert!(is_at_bottom(-1200., 1000., 240.));
    }

    #[test]
    fn the_three_gates_that_do_not_depend_on_scroll_position() {
        // 有下一页 + 列表非空 + 没在途 = 允许自动翻。
        assert!(auto_page_wanted(true, 30, false));
        // 少了"没在途"这一条，一帧渲染就能连发五个下一页；游标类分页（历史、图文）
        // 在途期间游标还没更新，那五个其实是同一个请求。
        assert!(!auto_page_wanted(true, 30, true));
        // 空列表时的"到底"是**还没加载**，不是滚到底，翻页只会原地打转。
        assert!(!auto_page_wanted(true, 0, false));
        // 接口说没有下一页就彻底停。
        assert!(!auto_page_wanted(false, 30, false));
    }

    #[test]
    fn auto_paging_stops_at_the_soft_cap_while_the_manual_button_still_works() {
        // 列表没有虚拟化（gpui 的 `List` 才有），每张卡片都是要布局与绘制的真元素：
        // 自动翻到几百条会把"按住拖动"拖垮，所以有一条总量闸。
        assert!(auto_page_wanted(true, AUTO_PAGE_MAX_ITEMS - 1, false));
        assert!(!auto_page_wanted(true, AUTO_PAGE_MAX_ITEMS, false));
        assert!(!auto_page_wanted(true, AUTO_PAGE_MAX_ITEMS + 500, false));
        // 上限也必须**明显大于一屏**，否则刚打开就翻不动了 —— 那是回归不是省资源。
        assert!(AUTO_PAGE_MAX_ITEMS >= 120, "{AUTO_PAGE_MAX_ITEMS}");
    }
}

#[cfg(test)]
mod assets_tests {
    use crate::ui::assets::AppAssets;
    use gpui_kit::AssetSource as _;
    use gpui_kit::assets::IconName;

    /// **我们代码里用到的每一个图标，都必须在这张表里**。
    ///
    /// 这条测试是"图标"这件事唯一的防护网：资源名找不到**不会编译失败、不会 panic**，
    /// 只会在界面上画出一片空白（第四轮踩过一次，那次是因为 kit 默认的 `Assets` 太少）。
    /// 现在换成自己那张 110 项的表，同一个坑的可能性反而更高了 —— 所以加图标时必须同时改这里：
    /// 新增一个 `IconName::X` 而没加进数组，这条测试就红，而不是等真机看出一片空白。
    #[test]
    fn every_icon_the_app_asks_for_is_embedded() {
        let used = [
            IconName::House,
            IconName::Search,
            IconName::Star,
            IconName::Clock,
            IconName::Bookmark,
            IconName::Download,
            IconName::Settings,
            IconName::ArrowLeft,
        ];
        for name in used {
            let path = name.path();
            assert!(AppAssets::has(path.as_ref()), "表里缺这个图标：{path}");
        }
    }

    /// 表里每一项都得真的能读出来（路径拼错一个字母，`load` 就会给 `None` → 界面一片空白）。
    #[test]
    fn each_table_entry_can_be_loaded() {
        for name in [
            IconName::House,
            IconName::Clock,
            IconName::Bookmark,
            IconName::Download,
            IconName::ArrowLeft,
            IconName::Star,
            IconName::Search,
            IconName::Settings,
        ] {
            let path = name.path();
            let bytes = AppAssets
                .load(path.as_ref())
                .expect("资源源本身不该报错")
                .expect("这条路径该读得出内容");
            assert!(
                bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml"),
                "{path} 读出来的不像 svg"
            );
        }
    }

    /// 体积回归闸：这张表存在的意义就是"别把整个 Lucide 目录塞进 exe"。
    /// 完整目录是 1830 个文件 / 2.8 MB（第四十八轮量的），我们这份要小一个数量级。
    #[test]
    fn the_embedded_table_stays_small() {
        assert_eq!(AppAssets::COUNT, 110, "图标数量变了要同时改这里（加/减图标是有意决定）");
        let total: usize = AppAssets::bytes();
        assert!(
            total < 600 * 1024,
            "图标表已经 {total} 字节了，快赶上整份 Lucide 目录的一半"
        );
    }
}
