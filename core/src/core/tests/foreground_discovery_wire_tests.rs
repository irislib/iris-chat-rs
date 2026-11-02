#[test]
fn endpoint_advert_wire_oracle_accepts_only_authorized_anonymous_public_reads() {
    let owner = Keys::generate();
    let device = Keys::generate();
    let siblings = [Keys::generate(), Keys::generate()];
    let peer = Keys::generate();
    let anonymous = Keys::generate();
    let own = [&owner, &device, &siblings[0], &siblings[1]]
        .into_iter()
        .map(|key| key.public_key().to_hex())
        .collect::<HashSet<_>>();
    let mut identities = own.clone();
    identities.insert(peer.public_key().to_hex());
    let profile = json!({"authors": [peer.public_key().to_hex()], "kinds": [0]});
    let stable = BTreeMap::from([(
        peer.public_key().to_hex(),
        BTreeSet::from([profile.to_string()]),
    )]);
    let advert_authors = [&peer, &siblings[0], &siblings[1]]
        .into_iter()
        .map(|key| key.public_key().to_hex())
        .collect::<BTreeSet<_>>();
    let advert = json!({"authors": advert_authors, "kinds": [37195]});
    let auth = |key: &Keys| {
        json!([
            "AUTH",
            EventBuilder::new(Kind::Authentication, "")
                .sign_with_keys(key)
                .unwrap()
        ])
    };
    let accepted = vec![
        (0, 1, auth(&anonymous)),
        (0, 1, json!(["REQ", "profiles", profile.clone()])),
        (0, 1, json!(["REQ", "adverts", advert.clone()])),
    ];
    let wire = Arc::new(Mutex::new(accepted.clone()));
    let (observed, history, recipients) =
        assert_wire(&wire, &own, &identities, &stable, &advert_authors);
    assert_eq!(observed, BTreeSet::from([(0, peer.public_key().to_hex())]));
    assert!(!history && !recipients);
    let with_cold_limits = |mut rows: Vec<(usize, usize, Value)>| {
        for (_, _, frame) in &mut rows {
            if frame[0] == "REQ" {
                for filter in &mut frame.as_array_mut().unwrap()[2..] {
                    filter
                        .as_object_mut()
                        .unwrap()
                        .entry("limit")
                        .or_insert(json!(16_384));
                }
            }
        }
        rows
    };
    let cold = Arc::new(Mutex::new(with_cold_limits(accepted.clone())));
    let (_, history, recipients) = assert_wire(&cold, &own, &identities, &stable, &advert_authors);
    assert!(!history && !recipients);
    let mut rejected = Vec::new();
    for forbidden in [
        owner.public_key(),
        device.public_key(),
        siblings[0].public_key(),
        Keys::generate().public_key(),
    ] {
        let companion = json!({"authors": [forbidden.to_hex()], "kinds": [0]});
        let mut same_request = accepted.clone();
        same_request[2]
            .2
            .as_array_mut()
            .unwrap()
            .push(companion.clone());
        rejected.push(same_request);
        for position in [1, accepted.len()] {
            let mut separate_request = accepted.clone();
            separate_request.insert(
                position,
                (0, 1, json!(["REQ", "companion", companion.clone()])),
            );
            rejected.push(separate_request);
        }
    }
    for extra in [json!({"until": 1}), json!({"#d": ["unrelated"]})] {
        let mut companion = profile.clone();
        companion
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let mut rows = accepted.clone();
        rows[2].2.as_array_mut().unwrap().push(companion);
        rejected.push(rows);
    }
    for forbidden in [
        owner.public_key(),
        device.public_key(),
        Keys::generate().public_key(),
    ] {
        let mut rows = accepted.clone();
        // Replace one authorized author, preserving the author count.
        rows[2].2[2]["authors"][0] = json!(forbidden.to_hex());
        rejected.push(rows);
    }
    for field in ["#p", "#d", "since", "until", "search", "unexpected"] {
        let mut rows = accepted.clone();
        rows[2].2[2][field] = if field.starts_with('#') {
            json!([owner.public_key().to_hex()])
        } else if field == "search" {
            json!("unapproved")
        } else {
            json!(1)
        };
        rejected.push(rows);
    }
    for limit in [
        json!(0),
        json!(1),
        json!(16_383),
        json!(16_385),
        json!(-1),
        json!("16384"),
        json!(null),
    ] {
        for request in [1, 2] {
            let mut rows = accepted.clone();
            rows[request].2[2]["limit"] = limit.clone();
            rejected.push(rows);
        }
    }
    let mut mixed_kinds = accepted.clone();
    mixed_kinds[2].2[2]["kinds"] = json!([0, 37195]);
    rejected.push(mixed_kinds);
    let mut mixed_profile = accepted.clone();
    mixed_profile[1].2[2]["authors"] = json!([
        peer.public_key().to_hex(),
        siblings[0].public_key().to_hex()
    ]);
    rejected.push(mixed_profile);
    for key in [&owner, &device, &siblings[0], &peer] {
        let mut rows = accepted.clone();
        rows[0].2 = auth(key);
        rejected.push(rows);
    }
    for private in [
        json!(["REQ", "private", {"kinds": [INVITE_RESPONSE_KIND], "#p": [Keys::generate().public_key().to_hex()]}]),
        json!(["REQ", "private", {"kinds": [MESSAGE_EVENT_KIND, 0], "authors": [Keys::generate().public_key().to_hex()]}]),
        json!([
            "EVENT",
            EventBuilder::new(Kind::TextNote, "unrelated publication")
                .sign_with_keys(&anonymous)
                .unwrap()
        ]),
    ] {
        let mut rows = accepted.clone();
        rows.push((0, 1, private));
        rejected.push(rows);
    }
    // Exercise every forbidden author, kind, tag, companion request,
    // identity and publication with the cold clamp too. Preserve explicit
    // invalid limits so normalization cannot conceal them.
    let cold_rejected = rejected
        .iter()
        .cloned()
        .map(with_cold_limits)
        .collect::<Vec<_>>();
    rejected.extend(cold_rejected);
    for (index, rows) in rejected.into_iter().enumerate() {
        let wire = Arc::new(Mutex::new(rows));
        assert!(
            std::panic::catch_unwind(|| {
                assert_wire(&wire, &own, &identities, &stable, &advert_authors);
            })
            .is_err(),
            "invalid discovery wire variant {index} was accepted"
        );
    }
    let combined = vec![
        (0, 1, auth(&anonymous)),
        (0, 1, json!(["REQ", "combined", advert, profile])),
    ];
    for rows in [combined.clone(), with_cold_limits(combined)] {
        let wire = Arc::new(Mutex::new(rows));
        assert_wire(&wire, &own, &identities, &stable, &advert_authors);
    }
}
