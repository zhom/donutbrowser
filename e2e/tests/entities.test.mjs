import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import test from "node:test";
import { withApp } from "../lib/app.mjs";
import {
  extensionZipBase64,
  wireGuardFixture,
  writeChromiumCookies,
  writeChromiumHistory,
} from "../lib/fixtures.mjs";

async function createProfile(app, name = "Entity Profile") {
  return app.invoke("create_browser_profile_new", {
    name,
    browserStr: "wayfern",
    version: "150.0.7871.100",
    releaseType: "stable",
    proxyId: null,
    vpnId: null,
    // CRUD-focused suites use a deterministic stored fingerprint. The browser
    // suite separately exercises real Wayfern fingerprint generation.
    wayfernConfig: { fingerprint: "{}" },
    groupId: null,
    ephemeral: false,
    dnsBlocklist: null,
    launchHook: null,
  });
}

test("profile, group, proxy, tag, metadata, clone, and bulk-delete lifecycle", async () => {
  // Profile import derives its browser version from the downloaded-browsers
  // registry, so without an entry every import fails with
  // BROWSER_NOT_DOWNLOADED before it touches a single file.
  await withApp(
    "entities-core",
    async (app) => {
      const group = await app.invoke("create_profile_group", {
        name: "Research",
      });
      assert.equal(group.name, "Research");
      const renamedGroup = await app.invoke("update_profile_group", {
        groupId: group.id,
        name: "Research Team",
      });
      assert.equal(renamedGroup.name, "Research Team");

      const duplicateError = await app.invokeError("create_profile_group", {
        name: "Research Team",
      });
      assert.match(duplicateError, /GROUP_ALREADY_EXISTS|already exists/i);

      const proxy = await app.invoke("create_stored_proxy", {
        name: "Local Dead Proxy",
        proxySettings: {
          proxy_type: "http",
          host: "127.0.0.1",
          port: 9,
          username: "e2e-user",
          password: "e2e-pass",
        },
      });
      assert.equal(proxy.proxy_settings.password, "e2e-pass");
      const updatedProxy = await app.invoke("update_stored_proxy", {
        proxyId: proxy.id,
        name: "Updated Proxy",
        proxySettings: {
          proxy_type: "socks5",
          host: "127.0.0.1",
          port: 9,
          username: null,
          password: null,
        },
      });
      assert.equal(updatedProxy.name, "Updated Proxy");
      assert.equal(updatedProxy.updated_at >= proxy.updated_at, true);

      const parsed = await app.invoke("parse_txt_proxies", {
        content: [
          "http://one.example:8080",
          "two.example:1080:user:pass",
          "not a proxy",
        ].join("\n"),
      });
      assert.equal(parsed.length, 3);
      assert.ok(parsed.some((result) => result.status === "parsed"));
      assert.ok(parsed.some((result) => result.status === "invalid"));
      const parsedProxy = parsed.find((result) => result.status === "parsed");
      const { status: _status, ...parsedProxyFields } = parsedProxy;
      const parsedImport = await app.invoke("import_proxies_from_parsed", {
        parsedProxies: [parsedProxyFields],
        namePrefix: "Parsed",
      });
      assert.equal(parsedImport.imported_count, 1);

      const validityError = await app.invokeError("check_proxy_validity", {
        proxyId: proxy.id,
        proxySettings: null,
      });
      assert.match(validityError, /Proxy check failed|Could not connect/i);
      const cachedValidity = await app.invoke("get_cached_proxy_check", {
        proxyId: proxy.id,
      });
      assert.ok(cachedValidity === null || cachedValidity.is_valid === false);

      // Donut accepts one VLESS shape (REALITY + XTLS Vision over TCP). The form
      // uses this to tell the user WHICH part of their setup is unsupported
      // instead of implying they mistyped, so the reason must survive the IPC hop.
      const goodVless =
        "vless://6d6e21a1-4829-4d2b-bc7f-1b25707b61e4@example.com:443" +
        "?security=reality&flow=xtls-rprx-vision&encryption=none&type=tcp" +
        "&sni=a.com&pbk=mQB9jxUDHO7g49VaNXLEdcNQ_jLhTbLolUsMUNwb6W4&sid=00&fp=chrome";
      assert.equal(
        await app.invoke("validate_vless_uri", { uri: goodVless }),
        null,
      );

      for (const [uri, reason] of [
        [goodVless.replace("security=reality", "security=tls"), "security"],
        [goodVless.replace("type=tcp", "type=ws"), "transport"],
        [goodVless.replace("flow=xtls-rprx-vision", "flow=none"), "flow"],
      ]) {
        // invokeError returns the command's error wrapped in a message, so match
        // rather than JSON.parse the whole string.
        const error = await app.invokeError("validate_vless_uri", { uri });
        assert.match(error, /VLESS_CONFIG_INVALID/);
        assert.match(
          error,
          new RegExp(`"reason":"${reason}"`),
          `expected reason ${reason} for ${uri}, got: ${error}`,
        );
      }

      const exported = JSON.parse(
        await app.invoke("export_proxies", { format: "json" }),
      );
      assert.equal(exported.proxies.length, 2);
      assert.ok(exported.proxies.some((item) => item.name === "Updated Proxy"));
      assert.ok(
        exported.proxies.some((item) => item.name === "Parsed Proxy 1"),
      );
      const importResult = await app.invoke("import_proxies_json", {
        content: JSON.stringify({
          version: "1",
          source: "Donut Browser",
          exported_at: new Date().toISOString(),
          proxies: [
            {
              name: "Imported Proxy",
              type: "http",
              host: "127.0.0.1",
              port: 8081,
            },
          ],
        }),
      });
      assert.equal(importResult.imported_count, 1);

      const profile = await createProfile(app);
      assert.equal(profile.name, "Entity Profile");
      assert.equal(
        (
          await app.invoke("update_profile_proxy", {
            profileId: profile.id,
            proxyId: proxy.id,
          })
        ).proxy_id,
        proxy.id,
      );
      await app.invoke("assign_profiles_to_group", {
        profileIds: [profile.id],
        groupId: group.id,
      });
      await app.invoke("rename_profile", {
        profileId: profile.id,
        newName: "Renamed Profile",
      });
      await app.invoke("update_profile_tags", {
        profileId: profile.id,
        tags: ["alpha", "automation"],
      });
      await app.invoke("update_profile_note", {
        profileId: profile.id,
        note: "Extensive E2E metadata",
      });
      await app.invoke("update_profile_window_color", {
        profileId: profile.id,
        windowColor: "#123456",
      });
      await app.invoke("update_profile_launch_hook", {
        profileId: profile.id,
        launchHook: `${process.env.DONUT_E2E_FIXTURE_URL}/launch-hook`,
      });
      const invalidHook = await app.invokeError("update_profile_launch_hook", {
        profileId: profile.id,
        launchHook: "file:///etc/passwd",
      });
      assert.match(invalidHook, /INVALID_LAUNCH_HOOK_URL/);
      await app.invoke("update_profile_proxy_bypass_rules", {
        profileId: profile.id,
        rules: ["localhost", "*.internal.example"],
      });
      await app.invoke("update_profile_dns_blocklist", {
        profileId: profile.id,
        dnsBlocklist: "light",
      });
      await app.invoke("update_profile_clear_on_close", {
        profileId: profile.id,
        clearOnClose: true,
      });

      const profiles = await app.invoke("list_browser_profiles");
      const changed = profiles.find((item) => item.id === profile.id);
      assert.deepEqual(changed.tags, ["alpha", "automation"]);
      assert.equal(changed.note, "Extensive E2E metadata");
      assert.equal(changed.window_color, "#123456");
      assert.equal(changed.group_id, group.id);
      assert.deepEqual(changed.proxy_bypass_rules, [
        "localhost",
        "*.internal.example",
      ]);
      assert.equal(changed.dns_blocklist, "light");
      assert.equal(changed.clear_on_close, true);
      assert.deepEqual((await app.invoke("get_all_tags")).sort(), [
        "alpha",
        "automation",
      ]);

      assert.ok(Array.isArray(await app.invoke("detect_existing_profiles")));
      const importRoot = path.join(app.root, "profile-import-fixture");
      const importProfile = path.join(importRoot, "Default");
      await mkdir(importProfile, { recursive: true });
      await writeFile(
        path.join(importProfile, "Preferences"),
        JSON.stringify({
          profile: { name: "Imported fixture", exit_type: "Crashed" },
          download: { default_directory: "/Users/someone-else/Downloads" },
        }),
      );
      // A Secure Preferences with MACs that can never validate under Wayfern,
      // one real (relative-path) extension and one component extension that
      // belongs to the source browser's bundle.
      await writeFile(
        path.join(importProfile, "Secure Preferences"),
        JSON.stringify({
          protection: { super_mac: "deadbeef", macs: { extensions: {} } },
          extensions: {
            settings: {
              aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa: {
                path: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/1.0_0",
              },
              bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb: {
                path: "/Applications/Chromium.app/Contents/Resources/component",
              },
            },
          },
        }),
      );
      // Caches must not be copied, and site data must be.
      await mkdir(path.join(importProfile, "Cache"), { recursive: true });
      await writeFile(path.join(importProfile, "Cache", "data_0"), "junk");
      await mkdir(path.join(importProfile, "Local Storage", "leveldb"), {
        recursive: true,
      });
      await writeFile(
        path.join(importProfile, "Local Storage", "leveldb", "000003.log"),
        "site-data",
      );
      writeChromiumHistory(path.join(importProfile, "History"), [
        "https://example.com/",
        "https://example.org/",
      ]);
      writeChromiumCookies(path.join(importProfile, "Cookies"), [
        { host: "example.com", name: "sid", value: "session-token" },
        { host: "example.org", name: "pref", value: "dark" },
        // Sealed with a key this machine does not have, and stored the way
        // Chromium's own v23->v24 migration stores it (TEXT in a BLOB column).
        // It must be reported as unrecoverable, never silently blanked and
        // counted as migrated.
        {
          host: "sealed.example",
          name: "sid",
          encryptedValueText: "v10\u0001\u0002\u0003unopenable-ciphertext",
        },
      ]);

      const scanned = await app.invoke("scan_folder_for_profiles", {
        folderPath: importRoot,
      });
      assert.equal(scanned.length, 1);
      assert.equal(scanned[0].mapped_browser, "wayfern");
      const importBatch = await app.invoke("import_browser_profiles", {
        items: [
          {
            source_path: scanned[0].path,
            browser_type: scanned[0].browser,
            new_profile_name: "Imported Profile",
            proxy_id: null,
            vpn_id: null,
          },
        ],
        groupId: null,
        duplicateStrategy: "rename",
        // A stored fingerprint, as elsewhere in this suite: generating a real
        // one shells out to the Wayfern binary, which no CRUD suite installs.
        wayfernConfig: { fingerprint: "{}" },
      });
      assert.equal(
        importBatch.imported_count,
        1,
        `import must succeed: ${JSON.stringify(importBatch.results)}`,
      );

      const imported = importBatch.results[0];
      // The assertion whose absence let the layout bug ship: an import that
      // carries nothing used to be indistinguishable from a successful one.
      assert.ok(
        imported.report,
        "an imported profile must report what it carried",
      );
      assert.equal(imported.report.cookies_migrated, 2);
      assert.equal(
        imported.report.cookies_unrecoverable,
        1,
        "a cookie no key can open must be counted, not silently emptied",
      );
      assert.equal(imported.report.history_entries, 2);
      assert.equal(imported.report.extensions_migrated, 1);
      assert.ok(imported.report.local_storage_origins > 0);

      const importedDir = path.join(
        app.dataRoot,
        "data",
        "profiles",
        imported.profile_id,
        "profile",
      );
      // Chromium reads <user-data-dir>/Default/, so anything at the root is
      // invisible to the browser no matter how faithfully it was copied.
      assert.ok(
        existsSync(path.join(importedDir, "Default", "Preferences")),
        "profile content must land under Default/",
      );
      assert.ok(
        !existsSync(path.join(importedDir, "Preferences")),
        "nothing profile-scoped may sit at the user-data-dir root",
      );
      assert.ok(
        existsSync(path.join(importedDir, "os_crypt_key")),
        "Wayfern reads its key from the user-data-dir root",
      );
      assert.ok(
        !existsSync(path.join(importedDir, "Default", "Cache")),
        "caches are pure waste and must not be copied",
      );
      assert.ok(
        existsSync(
          path.join(
            importedDir,
            "Default",
            "Local Storage",
            "leveldb",
            "000003.log",
          ),
        ),
        "site data must survive",
      );

      const importedCookies = path.join(
        importedDir,
        "Default",
        process.platform === "win32"
          ? path.join("Network", "Cookies")
          : "Cookies",
      );
      assert.ok(
        existsSync(importedCookies),
        "cookies must sit where this platform's Chromium reads them",
      );
      // Chromium drops any row where both value and encrypted_value are set, so
      // a "migrated" cookie that kept its plaintext would never load.
      const cookieDb = new DatabaseSync(importedCookies, { readOnly: true });
      const rows = cookieDb
        .prepare(
          "SELECT host_key, value, length(encrypted_value) AS enc FROM cookies ORDER BY host_key",
        )
        .all();
      cookieDb.close();
      assert.equal(
        rows.length,
        2,
        "the unrecoverable row is dropped, not kept empty",
      );
      for (const row of rows) {
        assert.equal(row.value, "", `${row.host_key} kept a plaintext value`);
        assert.ok(row.enc > 0, `${row.host_key} was not re-encrypted`);
      }

      const securePrefs = JSON.parse(
        await readFile(
          path.join(importedDir, "Default", "Secure Preferences"),
          "utf8",
        ),
      );
      assert.equal(
        securePrefs.protection,
        undefined,
        "MACs from another machine can never validate and must be stripped",
      );
      assert.ok(
        securePrefs.extensions.settings.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa,
        "the user's own extension must survive",
      );
      assert.equal(
        securePrefs.extensions.settings.bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb,
        undefined,
        "a component extension pointing into the source browser must be dropped",
      );

      const prefs = JSON.parse(
        await readFile(
          path.join(importedDir, "Default", "Preferences"),
          "utf8",
        ),
      );
      assert.equal(prefs.profile.exit_type, "Normal");
      assert.equal(prefs.download.default_directory, undefined);
      assert.equal(prefs.profile.name, "Imported fixture");

      // A Gecko profile must say why it cannot be imported instead of silently
      // producing an empty one.
      const firefoxRoot = path.join(app.root, "firefox-profile-fixture");
      await mkdir(firefoxRoot, { recursive: true });
      await writeFile(path.join(firefoxRoot, "prefs.js"), "// prefs");
      await writeFile(path.join(firefoxRoot, "places.sqlite"), "");
      const geckoBatch = await app.invoke("import_browser_profiles", {
        items: [
          {
            source_path: firefoxRoot,
            browser_type: "firefox",
            new_profile_name: "Gecko Profile",
            proxy_id: null,
            vpn_id: null,
          },
        ],
        groupId: null,
        duplicateStrategy: "rename",
        wayfernConfig: { fingerprint: "{}" },
      });
      assert.equal(geckoBatch.failed_count, 1);
      assert.match(
        geckoBatch.results[0].error,
        /IMPORT_SOURCE_NOT_CHROMIUM/,
        "a Firefox folder must be rejected by name, not imported empty",
      );
      const archivePath = path.join(app.root, "profile-import-fixture.zip");
      await writeFile(archivePath, Buffer.from(extensionZipBase64(), "base64"));
      const archiveScan = await app.invoke("scan_profile_archive", {
        archivePath,
      });
      assert.ok(Array.isArray(archiveScan.profiles));
      await app.invoke("cleanup_profile_import_scratch", {
        extractedDir: archiveScan.extracted_dir,
      });

      const clone = await app.invoke("clone_profile", {
        profileId: profile.id,
        name: "Cloned Profile",
      });
      assert.notEqual(clone.id, profile.id);
      assert.equal(clone.name, "Cloned Profile");
      const counts = await app.invoke("get_groups_with_profile_counts");
      assert.equal(counts.find((item) => item.id === group.id).count, 2);
      assert.equal((await app.invoke("get_profile_groups")).length, 1);

      await app.invoke("delete_selected_profiles", {
        profileIds: [profile.id, clone.id, imported.profile_id],
      });
      assert.deepEqual(await app.invoke("list_browser_profiles"), []);
      await app.invoke("delete_profile_group", { groupId: group.id });
      await app.invoke("delete_stored_proxy", { proxyId: proxy.id });
      for (const importedProxy of (
        await app.invoke("get_stored_proxies")
      ).filter(
        (item) =>
          item.name === "Imported Proxy" ||
          item.name.startsWith("Parsed Proxy"),
      )) {
        await app.invoke("delete_stored_proxy", { proxyId: importedProxy.id });
      }
    },
    { seedDownloadedBrowser: true },
  );
});

test("extensions, extension groups, VPN storage, DNS rules, and event-backed assignments", async () => {
  await withApp("entities-network-extension", async (app) => {
    const profile = await createProfile(app, "Assignment Profile");
    const extension = await app.invoke("add_extension", {
      name: "E2E Fixture Extension",
      fileName: "fixture.zip",
      fileData: [...Buffer.from(extensionZipBase64(), "base64")],
    });
    assert.equal(extension.name, "Donut E2E Fixture");
    assert.equal(extension.version, "1.0.0");
    const extensionGroup = await app.invoke("create_extension_group", {
      name: "Automation Extensions",
    });
    const populated = await app.invoke("add_extension_to_group", {
      groupId: extensionGroup.id,
      extensionId: extension.id,
    });
    assert.deepEqual(populated.extension_ids, [extension.id]);
    await app.invoke("assign_extension_group_to_profile", {
      profileId: profile.id,
      extensionGroupId: extensionGroup.id,
    });
    assert.equal(
      (
        await app.invoke("get_extension_group_for_profile", {
          profileId: profile.id,
        })
      ).id,
      extensionGroup.id,
    );
    const renamed = await app.invoke("update_extension", {
      extensionId: extension.id,
      name: "Renamed Fixture Extension",
      fileName: null,
      fileData: null,
    });
    assert.equal(renamed.name, "Renamed Fixture Extension");
    assert.equal(
      await app.invoke("get_extension_icon", { extensionId: extension.id }),
      null,
    );
    const changedGroup = await app.invoke("update_extension_group", {
      groupId: extensionGroup.id,
      name: "Renamed Extension Group",
      extensionIds: [extension.id],
    });
    assert.equal(changedGroup.name, "Renamed Extension Group");
    assert.equal((await app.invoke("list_extensions")).length, 1);
    assert.equal((await app.invoke("list_extension_groups")).length, 1);
    await app.invoke("remove_extension_from_group", {
      groupId: extensionGroup.id,
      extensionId: extension.id,
    });
    await app.invoke("assign_extension_group_to_profile", {
      profileId: profile.id,
      extensionGroupId: null,
    });
    await app.invoke("delete_extension_group", { groupId: extensionGroup.id });
    await app.invoke("delete_extension", { extensionId: extension.id });

    const vpn = await app.invoke("create_vpn_config_manual", {
      name: "E2E WireGuard",
      vpnType: "WireGuard",
      configData: wireGuardFixture(),
    });
    assert.equal(vpn.name, "E2E WireGuard");
    assert.equal(
      (await app.invoke("get_vpn_config", { vpnId: vpn.id })).id,
      vpn.id,
    );
    assert.equal((await app.invoke("list_vpn_configs")).length, 1);
    const updatedVpn = await app.invoke("update_vpn_config", {
      vpnId: vpn.id,
      name: "Updated WireGuard",
    });
    assert.equal(updatedVpn.name, "Updated WireGuard");
    assert.equal(
      (await app.invoke("get_vpn_status", { vpnId: vpn.id })).connected,
      false,
    );
    assert.equal(
      (
        await app.invoke("update_profile_vpn", {
          profileId: profile.id,
          vpnId: vpn.id,
        })
      ).vpn_id,
      vpn.id,
    );
    assert.deepEqual(await app.invoke("list_active_vpn_connections"), []);
    await app.invoke("disconnect_vpn", { vpnId: vpn.id });
    const unknownVpnError = await app.invokeError("check_vpn_validity", {
      vpnId: "missing-vpn",
    });
    const normalizedVpnError = unknownVpnError.toLowerCase();
    assert.ok(
      normalizedVpnError.includes("not found") ||
        normalizedVpnError.includes("failed to start vpn worker"),
    );
    const importedVpn = await app.invoke("import_vpn_config", {
      content: wireGuardFixture(),
      filename: "imported.conf",
      name: "Imported WireGuard",
    });
    assert.equal(importedVpn.success, true);
    await app.invoke("delete_vpn_config", { vpnId: importedVpn.vpn_id });
    await app.invoke("delete_vpn_config", { vpnId: vpn.id });

    const dns = await app.invoke("set_custom_dns_config", {
      sources: [`${process.env.DONUT_E2E_FIXTURE_URL}/dns.txt`],
      blockDomains: [" Ads.Example.com ", "tracker.example"],
      allowDomains: ["safe.example"],
      allowlistMode: false,
    });
    assert.deepEqual(dns.block_domains, ["ads.example.com", "tracker.example"]);
    assert.deepEqual(dns.allow_domains, ["safe.example"]);
    const textExport = await app.invoke("export_custom_dns_rules", {
      format: "txt",
    });
    assert.equal(
      textExport,
      [
        `! source: ${process.env.DONUT_E2E_FIXTURE_URL}/dns.txt`,
        "@@safe.example",
        "ads.example.com",
        "tracker.example",
        "",
      ].join("\n"),
    );
    await app.invoke("import_custom_dns_rules", {
      format: "txt",
      content: "||malware.example^\n@@||allowed.example^\n",
    });
    const importedDns = await app.invoke("get_custom_dns_config");
    assert.ok(importedDns.block_domains.includes("malware.example"));
    assert.ok(importedDns.allow_domains.includes("allowed.example"));
    await app.invoke("refresh_dns_blocklists");
    const blocklistStatus = await app.invoke("get_dns_blocklist_cache_status");
    assert.equal(blocklistStatus.length, 5);
    assert.ok(
      blocklistStatus.every(
        (entry) => entry.is_cached && entry.is_fresh && entry.entry_count === 2,
      ),
    );

    await app.invoke("delete_profile", { profileId: profile.id });
  });
});

test("cookie import/copy/export, profile encryption, and traffic-stat read/clear paths", async () => {
  await withApp("entities-cookies-password", async (app) => {
    const source = await createProfile(app, "Cookie Source");
    const target = await createProfile(app, "Cookie Target");
    const cookieJson = JSON.stringify([
      {
        name: "session",
        value: "isolated-secret-cookie",
        domain: "fixture.local",
        path: "/",
        secure: false,
        httpOnly: true,
        sameSite: "lax",
        expirationDate: 2_000_000_000,
      },
    ]);
    const imported = await app.invoke("import_cookies_from_file", {
      profileId: source.id,
      content: cookieJson,
    });
    assert.equal(imported.cookies_imported, 1);
    const cookies = await app.invoke("read_profile_cookies", {
      profileId: source.id,
    });
    assert.equal(cookies.total_count, 1);
    assert.equal(cookies.domains[0].cookies[0].value, "isolated-secret-cookie");
    const stats = await app.invoke("get_profile_cookie_stats", {
      profileId: source.id,
    });
    assert.equal(stats.total_count, 1);
    const copied = await app.invoke("copy_profile_cookies", {
      request: {
        source_profile_id: source.id,
        target_profile_ids: [target.id],
        selected_cookies: [{ domain: "fixture.local", name: "session" }],
      },
    });
    assert.equal(copied[0].cookies_copied, 1);
    assert.match(
      await app.invoke("export_profile_cookies", {
        profileId: target.id,
        format: "json",
      }),
      /isolated-secret-cookie/,
    );
    assert.match(
      await app.invoke("export_profile_cookies", {
        profileId: target.id,
        format: "netscape",
      }),
      /fixture\.local/,
    );

    await app.invoke("set_profile_password", {
      profileId: source.id,
      password: "correct horse battery staple",
    });
    assert.equal(
      await app.invoke("is_profile_locked", { profileId: source.id }),
      false,
    );
    const wrong = await app.invokeError("verify_profile_password", {
      profileId: source.id,
      password: "wrong password",
    });
    assert.match(wrong, /INCORRECT_PASSWORD/);
    await app.invoke("verify_profile_password", {
      profileId: source.id,
      password: "correct horse battery staple",
    });
    await app.invoke("change_profile_password", {
      profileId: source.id,
      oldPassword: "correct horse battery staple",
      newPassword: "new correct horse battery staple",
    });
    await app.invoke("lock_profile", { profileId: source.id });
    assert.equal(
      await app.invoke("is_profile_locked", { profileId: source.id }),
      true,
    );
    await app.invoke("unlock_profile", {
      profileId: source.id,
      password: "new correct horse battery staple",
    });
    await app.invoke("remove_profile_password", {
      profileId: source.id,
      password: "new correct horse battery staple",
    });
    assert.equal(
      await app.invoke("is_profile_locked", { profileId: source.id }),
      false,
    );

    assert.deepEqual(await app.invoke("get_all_traffic_snapshots"), []);
    assert.equal(
      await app.invoke("get_profile_traffic_snapshot", {
        profileId: source.id,
      }),
      null,
    );
    assert.equal(
      await app.invoke("get_traffic_stats_for_period", {
        profileId: source.id,
        seconds: 3600,
      }),
      null,
    );
    await app.invoke("clear_profile_traffic_stats", { profileId: source.id });
    await app.invoke("clear_all_traffic_stats");

    await app.invoke("delete_selected_profiles", {
      profileIds: [source.id, target.id],
    });
  });
});
