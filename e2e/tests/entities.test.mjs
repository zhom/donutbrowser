import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import {
  mkdir,
  readdir,
  readFile,
  realpath,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import test from "node:test";
import { withApp } from "../lib/app.mjs";
import {
  CRX_EXTENSION_NAME,
  CRX_EXTENSION_VERSION,
  extensionIconPngBase64,
  extensionZipBase64,
  wireGuardFixture,
  writeChromiumCookies,
  writeChromiumHistory,
  writeUnpackedExtension,
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

      assert.deepEqual(
        await app.invoke("get_proxy_check_history", { proxyId: proxy.id }),
        [],
        "a proxy nobody has checked has no trail",
      );
      const validityError = await app.invokeError("check_proxy_validity", {
        proxyId: proxy.id,
        proxySettings: null,
      });
      assert.match(validityError, /Proxy check failed|Could not connect/i);
      const cachedValidity = await app.invoke("get_cached_proxy_check", {
        proxyId: proxy.id,
      });
      assert.ok(cachedValidity === null || cachedValidity.is_valid === false);

      // A check that failed is still a check, and it is recorded as one. The
      // proxy above was edited to SOCKS5 on a closed port, so the UDP probe
      // could not reach it: the honest verdict is "unknown", never "no".
      const trail = await app.invoke("get_proxy_check_history", {
        proxyId: proxy.id,
      });
      assert.equal(trail.length, 1);
      assert.equal(trail[0].ok, false);
      assert.equal(trail[0].ip, null);
      assert.equal(trail[0].udp, "unknown");
      assert.ok(
        typeof trail[0].latency_ms === "number" && trail[0].latency_ms >= 0,
      );
      assert.ok(trail[0].timestamp > 0);

      // Deleting the proxy takes the trail with it; it names exit addresses.
      const doomed = await app.invoke("create_stored_proxy", {
        name: "Trail Owner",
        proxySettings: {
          proxy_type: "http",
          host: "127.0.0.1",
          port: 9,
          username: null,
          password: null,
        },
      });
      await app.invokeError("check_proxy_validity", {
        proxyId: doomed.id,
        proxySettings: null,
      });
      const doomedTrail = await app.invoke("get_proxy_check_history", {
        proxyId: doomed.id,
      });
      assert.equal(doomedTrail.length, 1);
      // An HTTP proxy cannot carry a datagram at all, which is answered from
      // the protocol without dialling anything.
      assert.equal(doomedTrail[0].udp, "no");
      await app.invoke("delete_stored_proxy", { proxyId: doomed.id });
      assert.deepEqual(
        await app.invoke("get_proxy_check_history", { proxyId: doomed.id }),
        [],
      );

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

    // Folder imports, the "Load unpacked" flow. Copying packs the folder into
    // the store; linking loads it from where the user keeps it, which only
    // exists on this machine and therefore never syncs.
    const unpackedDir = await writeUnpackedExtension(
      path.join(app.root, "fixtures", "unpacked-extension"),
    );
    const copied = await app.invoke("add_unpacked_extension", {
      name: "Overridden By The Manifest",
      path: unpackedDir,
      link: false,
    });
    assert.equal(copied.source_kind, "unpacked");
    assert.equal(copied.linked_path, null);
    assert.equal(copied.file_type, "zip");
    assert.equal(copied.file_name, "unpacked-extension.zip");
    assert.equal(copied.name, "Donut E2E Unpacked");
    assert.equal(copied.version, "1.0.0");
    // The folder declares icons, so packing it must carry one through into the
    // store rather than dropping it the way the icon-less ZIP fixture does.
    assert.equal(
      await app.invoke("get_extension_icon", { extensionId: copied.id }),
      `data:image/png;base64,${extensionIconPngBase64()}`,
    );

    const linked = await app.invoke("add_unpacked_extension", {
      name: "Linked Fixture",
      path: unpackedDir,
      link: true,
    });
    assert.equal(linked.source_kind, "unpacked");
    assert.equal(linked.file_type, "unpacked");
    assert.equal(linked.linked_path, await realpath(unpackedDir));
    assert.equal(
      linked.sync_enabled,
      false,
      "a linked extension has no payload to upload, so it must never be synced",
    );

    const repackedDir = await writeUnpackedExtension(
      path.join(app.root, "fixtures", "unpacked-extension-v2"),
      { name: "Donut E2E Unpacked v2", version: "2.0.0" },
    );
    const repacked = await app.invoke("update_extension_from_path", {
      extensionId: copied.id,
      name: "Repacked Fixture Extension",
      path: repackedDir,
      link: false,
    });
    assert.equal(repacked.name, "Repacked Fixture Extension");
    assert.equal(repacked.version, "2.0.0");
    assert.equal(repacked.file_name, "unpacked-extension-v2.zip");
    assert.deepEqual(
      await readdir(
        path.join(app.dataRoot, "data", "extensions", copied.id, "file"),
      ),
      ["unpacked-extension-v2.zip"],
      "re-importing replaces the stored payload instead of stacking a second one",
    );

    // Re-importing a linked extension as a copy ends the link, which is what
    // makes it portable again. With no explicit name the manifest names it.
    const unlinked = await app.invoke("update_extension_from_path", {
      extensionId: linked.id,
      name: null,
      path: repackedDir,
      link: false,
    });
    assert.equal(unlinked.linked_path, null);
    assert.equal(unlinked.source_kind, "unpacked");
    assert.equal(unlinked.name, "Donut E2E Unpacked v2");

    assert.match(
      await app.invokeError("add_unpacked_extension", {
        name: "Not An Extension",
        path: app.root,
        link: false,
      }),
      /EXTENSION_MANIFEST_MISSING/,
    );
    assert.match(
      await app.invokeError("update_extension_from_path", {
        extensionId: unlinked.id,
        name: null,
        path: path.join(app.root, "fixtures", "absent"),
        link: false,
      }),
      /EXTENSION_DIR_NOT_FOUND/,
    );

    for (const id of [copied.id, unlinked.id]) {
      await app.invoke("delete_extension", { extensionId: id });
    }
    assert.deepEqual(await app.invoke("list_extensions"), []);
    assert.ok(
      existsSync(path.join(unpackedDir, "manifest.json")),
      "importing a folder must never move or consume the user's copy of it",
    );

    // Importing from a link. The fixture server answers with a real CRX3
    // container, so this proves the importer unwraps the signed container to
    // the ZIP the store keeps rather than filing the container itself.
    const fixtureBase = process.env.DONUT_E2E_FIXTURE_URL;
    assert.ok(fixtureBase, "the fixture server URL has to reach the suite");
    const fetched = await app.invoke("fetch_extension_from_url", {
      url: `${fixtureBase}/extension.crx`,
    });
    assert.equal(fetched.name, CRX_EXTENSION_NAME);
    assert.equal(fetched.version, CRX_EXTENSION_VERSION);
    assert.equal(fetched.from_web_store, false);
    assert.equal(
      fetched.file_name,
      "extension.zip",
      "the stored payload is the ZIP, so it must not still be called a .crx",
    );
    assert.deepEqual(
      fetched.file_data.slice(0, 4),
      [0x50, 0x4b, 0x03, 0x04],
      "the CRX3 header has to be stripped, not stored",
    );

    const fromLink = await app.invoke("add_extension", {
      name: "Overridden By The Manifest",
      fileName: fetched.file_name,
      fileData: fetched.file_data,
    });
    assert.equal(fromLink.name, CRX_EXTENSION_NAME);
    assert.equal(fromLink.version, CRX_EXTENSION_VERSION);
    assert.equal(fromLink.source_kind, "archive");
    assert.equal(fromLink.file_type, "zip");

    // Assignable like any other extension: the link is only how it arrived.
    const linkGroup = await app.invoke("create_extension_group", {
      name: "Downloaded Extensions",
    });
    assert.deepEqual(
      (
        await app.invoke("add_extension_to_group", {
          groupId: linkGroup.id,
          extensionId: fromLink.id,
        })
      ).extension_ids,
      [fromLink.id],
    );
    await app.invoke("assign_extension_group_to_profile", {
      profileId: profile.id,
      extensionGroupId: linkGroup.id,
    });
    assert.equal(
      (
        await app.invoke("get_extension_group_for_profile", {
          profileId: profile.id,
        })
      ).id,
      linkGroup.id,
    );

    // A body that is not an extension is refused with the code, and nothing
    // is stored for it.
    assert.match(
      await app.invokeError("fetch_extension_from_url", {
        url: `${fixtureBase}/not-an-extension.zip`,
      }),
      /EXTENSION_NOT_AN_EXTENSION/,
    );
    for (const rejected of [
      "not a link at all",
      "https://example.invalid/downloads",
      "https://example.invalid/installer.exe",
      // 32 characters, but an extension id only uses a-p.
      "abcdefghijklmnopabcdefghijklmnoz",
      // Plain HTTP off loopback never crosses the wire, whatever it points at.
      "http://files.example.invalid/pack.crx",
    ]) {
      assert.match(
        await app.invokeError("fetch_extension_from_url", { url: rejected }),
        /EXTENSION_URL_INVALID/,
        rejected,
      );
    }
    assert.match(
      await app.invokeError("fetch_extension_from_url", {
        url: `${fixtureBase}/absent-extension.crx`,
      }),
      /EXTENSION_NOT_AN_EXTENSION|EXTENSION_DOWNLOAD_FAILED/,
    );
    assert.equal((await app.invoke("list_extensions")).length, 1);

    await app.invoke("assign_extension_group_to_profile", {
      profileId: profile.id,
      extensionGroupId: null,
    });
    await app.invoke("delete_extension_group", { groupId: linkGroup.id });
    await app.invoke("delete_extension", { extensionId: fromLink.id });
    assert.deepEqual(await app.invoke("list_extensions"), []);

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
    const imported = await app.invoke("import_pasted_cookies", {
      profileId: source.id,
      content: cookieJson,
      site: null,
      mode: "merge",
      includeExpired: false,
    });
    assert.equal(imported.added, 1);
    assert.equal(imported.overwritten, 0);
    assert.equal(imported.deleted, 0);
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

    const paste = [
      "# Netscape HTTP Cookie File",
      "#HttpOnly_.fixture.local\tTRUE\t/\tFALSE\t2000000000\tpasted\tpasted-value",
    ].join("\n");
    const analysis = await app.invoke("analyze_pasted_cookies", {
      profileId: target.id,
      content: paste,
      site: null,
    });
    assert.equal(analysis.format, "netscape");
    assert.equal(analysis.cookies.length, 1);
    assert.equal(analysis.cookies[0].name, "pasted");
    assert.equal(analysis.cookies[0].isHttpOnly, true);
    assert.equal(
      analysis.cookies[0].value,
      undefined,
      "the preview must never carry the cookie value",
    );
    assert.equal(analysis.siteRequired, false);
    assert.equal(analysis.expiredCount, 0);
    assert.equal(analysis.blockedBy, null);
    // The copied fixture.local cookie is the one row replace mode would clear.
    assert.equal(analysis.replaceDeleteCount, 1);

    const merged = await app.invoke("import_pasted_cookies", {
      profileId: target.id,
      content: paste,
      site: null,
      mode: "merge",
      includeExpired: false,
    });
    assert.equal(merged.added, 1);
    assert.equal(merged.deleted, 0);
    assert.equal(merged.skipped, 0);
    assert.equal(
      (await app.invoke("get_profile_cookie_stats", { profileId: target.id }))
        .total_count,
      2,
    );

    // Both spellings of the pasted site go, and only they do.
    const replacedPaste = await app.invoke("import_pasted_cookies", {
      profileId: target.id,
      content: paste,
      site: null,
      mode: "replaceMatchingSites",
      includeExpired: false,
    });
    assert.equal(replacedPaste.deleted, 2);
    assert.equal(replacedPaste.added, 1);
    const afterReplace = await app.invoke("read_profile_cookies", {
      profileId: target.id,
    });
    assert.equal(afterReplace.total_count, 1);
    assert.equal(afterReplace.domains[0].cookies[0].name, "pasted");

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

test("deleted profiles land in the trash and come back intact on restore", async () => {
  await withApp("entities-trash", async (app) => {
    const initialSettings = await app.invoke("get_app_settings");
    assert.equal(initialSettings.trash_retention_days, 30);
    const savedSettings = await app.invoke("save_app_settings", {
      settings: { ...initialSettings, trash_retention_days: 7 },
    });
    assert.equal(savedSettings.trash_retention_days, 7);
    assert.equal(
      (await app.invoke("get_app_settings")).trash_retention_days,
      7,
    );
    // Out-of-range values are clamped, never rejected.
    const clamped = await app.invoke("save_app_settings", {
      settings: { ...savedSettings, trash_retention_days: 9000 },
    });
    assert.equal(clamped.trash_retention_days, 365);
    await app.invoke("save_app_settings", {
      settings: { ...clamped, trash_retention_days: 7 },
    });

    assert.deepEqual(await app.invoke("list_trashed_profiles"), []);

    const group = await app.invoke("create_profile_group", {
      name: "Trash Group",
    });
    const created = await app.invoke("create_browser_profile_new", {
      name: "Recoverable",
      browserStr: "wayfern",
      version: "150.0.7871.100",
      releaseType: "stable",
      proxyId: null,
      vpnId: null,
      wayfernConfig: {
        fingerprint: "{}",
        identity_id: "identity-e2e",
        identity_overrides: JSON.stringify({ userAgent: "Custom UA" }),
        location: JSON.stringify({
          timezone: "Europe/Berlin",
          language: "de-DE",
        }),
      },
      groupId: group.id,
      ephemeral: false,
      dnsBlocklist: null,
      launchHook: null,
    });
    await app.invoke("update_profile_tags", {
      profileId: created.id,
      tags: ["shop", "eu"],
    });
    const before = (await app.invoke("list_browser_profiles")).find(
      (item) => item.id === created.id,
    );
    assert.equal(before.wayfern_config.identity_id, "identity-e2e");
    assert.deepEqual(before.tags, ["shop", "eu"]);
    assert.equal(before.group_id, group.id);

    // Real files to carry through the move, plus a cache the trash must drop.
    const profilesDir = path.join(app.dataRoot, "data", "profiles");
    const dataDir = path.join(profilesDir, created.id, "profile");
    await mkdir(path.join(dataDir, "Default"), { recursive: true });
    await writeFile(path.join(dataDir, "Default", "Cookies"), "cookie-db");
    await mkdir(path.join(dataDir, "Cache"), { recursive: true });
    await writeFile(path.join(dataDir, "Cache", "blob"), "cache-bytes");

    await app.invoke("delete_profile", { profileId: created.id });
    assert.equal(
      (await app.invoke("list_browser_profiles")).some(
        (item) => item.id === created.id,
      ),
      false,
    );
    const trashed = await app.invoke("list_trashed_profiles");
    assert.equal(trashed.length, 1);
    assert.equal(trashed[0].id, created.id);
    assert.equal(trashed[0].name, "Recoverable");
    assert.equal(trashed[0].browser, "wayfern");
    assert.equal(trashed[0].version, "150.0.7871.100");
    assert.equal(trashed[0].group_id, group.id);
    assert.equal(trashed[0].password_protected, false);
    assert.equal(
      trashed[0].expires_at - trashed[0].deleted_at,
      7 * 24 * 60 * 60,
    );
    assert.ok(trashed[0].size_bytes > 0);
    const trashDir = path.join(app.dataRoot, "data", "trash");
    const entryDir = path.join(trashDir, created.id);
    assert.ok(existsSync(path.join(entryDir, "profile.json")));
    assert.ok(existsSync(path.join(entryDir, "manifest.json")));
    assert.equal(
      await readFile(
        path.join(entryDir, "profile", "Default", "Cookies"),
        "utf8",
      ),
      "cookie-db",
    );
    assert.equal(
      existsSync(path.join(entryDir, "profile", "Cache")),
      false,
      "caches are pruned before the move",
    );
    assert.equal(existsSync(path.join(profilesDir, created.id)), false);

    // A live profile carrying the same name pushes the restored one to a suffix.
    const namesake = await createProfile(app, "Recoverable");
    const restored = await app.invoke("restore_trashed_profile", {
      profileId: created.id,
    });
    assert.equal(restored.id, created.id);
    assert.equal(restored.name, "Recoverable (restored)");
    assert.deepEqual(restored.wayfern_config, before.wayfern_config);
    assert.deepEqual(restored.tags, before.tags);
    assert.equal(restored.group_id, group.id);
    assert.ok(restored.updated_at >= (before.updated_at ?? 0));
    const live = (await app.invoke("list_browser_profiles")).find(
      (item) => item.id === created.id,
    );
    assert.deepEqual(live.wayfern_config, before.wayfern_config);
    assert.deepEqual(live.tags, before.tags);
    assert.equal(
      await readFile(
        path.join(profilesDir, created.id, "profile", "Default", "Cookies"),
        "utf8",
      ),
      "cookie-db",
    );
    assert.equal(existsSync(entryDir), false);
    assert.deepEqual(await app.invoke("list_trashed_profiles"), []);
    assert.match(
      await app.invokeError("restore_trashed_profile", {
        profileId: created.id,
      }),
      /TRASH_ENTRY_NOT_FOUND/,
    );

    // A group deleted while the profile sat in the trash is not resurrected.
    await app.invoke("delete_profile", { profileId: created.id });
    await app.invoke("delete_profile_group", { groupId: group.id });
    const restoredWithoutGroup = await app.invoke("restore_trashed_profile", {
      profileId: created.id,
    });
    assert.equal(restoredWithoutGroup.id, created.id);
    assert.equal(restoredWithoutGroup.group_id, null);
    assert.deepEqual(
      restoredWithoutGroup.wayfern_config,
      before.wayfern_config,
    );

    // Delete again, then purge: gone for good.
    await app.invoke("delete_profile", { profileId: created.id });
    assert.equal((await app.invoke("list_trashed_profiles")).length, 1);
    await app.invoke("purge_trashed_profile", { profileId: created.id });
    assert.deepEqual(await app.invoke("list_trashed_profiles"), []);
    assert.equal(existsSync(entryDir), false);
    assert.equal(existsSync(path.join(profilesDir, created.id)), false);
    assert.match(
      await app.invokeError("purge_trashed_profile", {
        profileId: created.id,
      }),
      /TRASH_ENTRY_NOT_FOUND/,
    );

    // An explicit permanent delete never lands in the trash.
    const doomed = await createProfile(app, "Doomed");
    await app.invoke("delete_profile", {
      profileId: doomed.id,
      permanent: true,
    });
    assert.deepEqual(await app.invoke("list_trashed_profiles"), []);
    assert.equal(existsSync(path.join(trashDir, doomed.id)), false);
    assert.equal(existsSync(path.join(profilesDir, doomed.id)), false);

    // A bulk delete trashes every profile; emptying the trash clears them all.
    const bulkA = await createProfile(app, "Bulk A");
    const bulkB = await createProfile(app, "Bulk B");
    await app.invoke("delete_selected_profiles", {
      profileIds: [bulkA.id, bulkB.id],
    });
    assert.deepEqual(
      (await app.invoke("list_trashed_profiles"))
        .map((entry) => entry.name)
        .sort(),
      ["Bulk A", "Bulk B"],
    );
    assert.equal(await app.invoke("empty_trash"), 2);
    assert.deepEqual(await app.invoke("list_trashed_profiles"), []);
    assert.equal(existsSync(path.join(trashDir, bulkA.id)), false);

    // Restore refuses an entry whose id a live profile already carries.
    const conflictDir = path.join(trashDir, namesake.id);
    await mkdir(conflictDir, { recursive: true });
    await writeFile(
      path.join(conflictDir, "profile.json"),
      JSON.stringify(namesake),
    );
    await writeFile(
      path.join(conflictDir, "manifest.json"),
      JSON.stringify({
        deleted_at: 1,
        expires_at: 4_102_444_800,
        size_bytes: 0,
        original_name: namesake.name,
      }),
    );
    assert.match(
      await app.invokeError("restore_trashed_profile", {
        profileId: namesake.id,
      }),
      /TRASH_RESTORE_CONFLICT/,
    );
    await app.invoke("purge_trashed_profile", { profileId: namesake.id });
    assert.deepEqual(await app.invoke("list_trashed_profiles"), []);

    await app.invoke("delete_profile", {
      profileId: namesake.id,
      permanent: true,
    });
    assert.deepEqual(await app.invoke("list_browser_profiles"), []);
  });
});

test("proxies distribute one to one, and group bookmarks reach the profile's Bookmarks file", async () => {
  await withApp("entities-distribution-bookmarks", async (app) => {
    const profiles = [];
    for (const name of ["Fleet 1", "Fleet 2", "Fleet 3", "Fleet 4"]) {
      profiles.push(await createProfile(app, name));
    }
    const proxies = [];
    for (const [index, name] of ["Exit A", "Exit B", "Exit C"].entries()) {
      proxies.push(
        await app.invoke("create_stored_proxy", {
          name,
          proxySettings: {
            proxy_type: "http",
            host: "127.0.0.1",
            port: 9001 + index,
            username: null,
            password: null,
          },
        }),
      );
    }

    const profileIds = profiles.map((profile) => profile.id);
    const proxyIds = proxies.map((proxy) => proxy.id);

    // Four profiles, three proxies: three pairs and one profile left alone.
    // The fourth must NEVER wrap around onto the first proxy.
    const plan = await app.invoke("plan_proxy_distribution", {
      profileIds,
      proxyIds,
      allowSharing: false,
    });
    assert.deepEqual(
      plan.pairs,
      proxyIds.map((proxyId, index) => ({
        profile_id: profileIds[index],
        proxy_id: proxyId,
      })),
    );
    assert.deepEqual(plan.unpaired_profile_ids, [profileIds[3]]);
    assert.deepEqual(plan.unused_proxy_ids, []);
    assert.deepEqual(plan.shared_proxy_ids, []);
    assert.deepEqual(plan.running_profile_ids, []);

    const results = await app.invoke("distribute_proxies_to_profiles", {
      pairs: plan.pairs,
    });
    assert.equal(results.length, 3);
    assert.ok(results.every((result) => result.ok));

    const afterDistribution = await app.invoke("list_browser_profiles");
    const proxyOf = (id) =>
      afterDistribution.find((profile) => profile.id === id).proxy_id;
    assert.equal(proxyOf(profileIds[0]), proxyIds[0]);
    assert.equal(proxyOf(profileIds[1]), proxyIds[1]);
    assert.equal(proxyOf(profileIds[2]), proxyIds[2]);
    assert.equal(proxyOf(profileIds[3]) ?? null, null);

    // A proxy someone else holds is refused by default and only offered once
    // the caller asks for sharing explicitly.
    const strict = await app.invoke("plan_proxy_distribution", {
      profileIds: [profileIds[3]],
      proxyIds: [proxyIds[0]],
      allowSharing: false,
    });
    assert.deepEqual(strict.pairs, []);
    assert.deepEqual(strict.shared_proxy_ids, [proxyIds[0]]);
    assert.deepEqual(strict.unpaired_profile_ids, [profileIds[3]]);

    const permissive = await app.invoke("plan_proxy_distribution", {
      profileIds: [profileIds[3]],
      proxyIds: [proxyIds[0]],
      allowSharing: true,
    });
    assert.deepEqual(permissive.pairs, [
      { profile_id: profileIds[3], proxy_id: proxyIds[0] },
    ]);

    // Per-profile failures never break the batch: one good pair still lands.
    const mixed = await app.invoke("distribute_proxies_to_profiles", {
      pairs: [
        { profile_id: profileIds[3], proxy_id: proxyIds[0] },
        { profile_id: profileIds[3], proxy_id: proxyIds[1] },
        {
          profile_id: profileIds[0],
          proxy_id: "00000000-0000-4000-8000-000000000000",
        },
      ],
    });
    assert.equal(mixed[0].ok, true);
    assert.equal(mixed[1].ok, false);
    assert.match(mixed[1].error, /PROFILE_PAIRED_TWICE/);
    assert.equal(mixed[2].ok, false);
    assert.match(mixed[2].error, /PROXY_NOT_FOUND/);
    assert.equal(
      (await app.invoke("list_browser_profiles")).find(
        (profile) => profile.id === profileIds[3],
      ).proxy_id,
      proxyIds[0],
    );

    // --- group bookmarks ---
    const group = await app.invoke("create_profile_group", {
      name: "Client Sites",
    });
    assert.deepEqual(
      await app.invoke("get_group_bookmarks", { groupId: group.id }),
      [],
    );

    const refused = await app.invokeError("set_group_bookmarks", {
      groupId: group.id,
      bookmarks: [{ title: "Keys", url: "file:///etc/passwd", folder: null }],
    });
    assert.match(refused, /URL_SCHEME_NOT_ALLOWED/);
    const unnamed = await app.invokeError("set_group_bookmarks", {
      groupId: group.id,
      bookmarks: [{ title: "  ", url: "https://ok.example", folder: null }],
    });
    assert.match(unnamed, /NAME_CANNOT_BE_EMPTY/);

    const saved = await app.invoke("set_group_bookmarks", {
      groupId: group.id,
      bookmarks: [
        { title: "Support", url: "https://support.example", folder: null },
        { title: "Console", url: "https://console.example", folder: "Ops" },
      ],
    });
    assert.equal(saved.length, 2);
    assert.equal(saved[1].folder, "Ops");
    assert.equal(
      (await app.invoke("get_groups_with_profile_counts")).find(
        (item) => item.id === group.id,
      ).bookmark_count,
      2,
    );

    const target = profiles[0];
    await app.invoke("assign_profiles_to_group", {
      profileIds: [target.id],
      groupId: group.id,
    });

    // Seed the profile's own Bookmarks file the way a real Chromium session
    // would have left it, so the write has something of the user's to preserve.
    const bookmarksFile = path.join(
      app.dataRoot,
      "data",
      "profiles",
      target.id,
      "profile",
      "Default",
      "Bookmarks",
    );
    await mkdir(path.dirname(bookmarksFile), { recursive: true });
    const permanentFolder = (id, name) => ({
      children: [],
      date_added: "13300000000000000",
      date_modified: "13300000000000000",
      guid: `0000000${id}-0000-4000-8000-000000000000`,
      id: String(id),
      name,
      type: "folder",
    });
    await writeFile(
      bookmarksFile,
      JSON.stringify({
        checksum: "0".repeat(32),
        roots: {
          bookmark_bar: {
            ...permanentFolder(1, "Bookmarks bar"),
            children: [
              {
                date_added: "13300000000000000",
                guid: "aaaaaaaa-0000-4000-8000-000000000000",
                id: "9",
                name: "My Bank",
                type: "url",
                url: "https://bank.example/",
              },
            ],
          },
          other: permanentFolder(2, "Other bookmarks"),
          synced: permanentFolder(3, "Mobile bookmarks"),
        },
        sync_metadata: "Zm9v",
        version: 1,
      }),
    );

    const readBookmarks = async () =>
      JSON.parse(await readFile(bookmarksFile, "utf8"));
    const managedFolderOf = (document) =>
      document.roots.bookmark_bar.children.filter(
        (child) =>
          child.type === "folder" &&
          child.meta_info?.donut_managed_group_bookmarks === "1",
      );

    assert.equal(
      await app.invoke("apply_group_bookmarks_to_profile", {
        profileId: target.id,
      }),
      true,
    );

    let document = await readBookmarks();
    let managed = managedFolderOf(document);
    assert.equal(managed.length, 1);
    assert.equal(managed[0].name, "Donut Group Bookmarks");
    assert.deepEqual(
      managed[0].children.map((child) => child.name),
      ["Support", "Ops"],
    );
    assert.deepEqual(
      managed[0].children[1].children.map((child) => child.url),
      ["https://console.example"],
    );
    // The user's own bookmark, the other roots and Chromium's opaque state all
    // survive; only the checksum is rewritten to describe the new tree.
    assert.equal(document.roots.bookmark_bar.children[0].name, "My Bank");
    assert.equal(document.sync_metadata, "Zm9v");
    assert.equal(document.version, 1);
    assert.notEqual(document.checksum, "0".repeat(32));
    assert.match(document.checksum, /^[0-9a-f]{32}$/);

    // Applying again is a no-op: the folder is not duplicated and the file is
    // not even rewritten.
    const firstWrite = await readFile(bookmarksFile, "utf8");
    assert.equal(
      await app.invoke("apply_group_bookmarks_to_profile", {
        profileId: target.id,
      }),
      false,
    );
    assert.equal(await readFile(bookmarksFile, "utf8"), firstWrite);

    // Removing a bookmark from the group removes it from the folder next time.
    await app.invoke("set_group_bookmarks", {
      groupId: group.id,
      bookmarks: [
        { title: "Support", url: "https://support.example", folder: null },
      ],
    });
    assert.equal(
      await app.invoke("apply_group_bookmarks_to_profile", {
        profileId: target.id,
      }),
      true,
    );
    document = await readBookmarks();
    managed = managedFolderOf(document);
    assert.equal(managed.length, 1);
    assert.deepEqual(
      managed[0].children.map((child) => child.name),
      ["Support"],
    );

    // Emptying the group takes the whole folder away and leaves the user's own.
    await app.invoke("set_group_bookmarks", {
      groupId: group.id,
      bookmarks: [],
    });
    assert.equal(
      await app.invoke("apply_group_bookmarks_to_profile", {
        profileId: target.id,
      }),
      true,
    );
    document = await readBookmarks();
    assert.deepEqual(managedFolderOf(document), []);
    assert.deepEqual(
      document.roots.bookmark_bar.children.map((child) => child.name),
      ["My Bank"],
    );

    // A profile in no group is left entirely alone.
    assert.equal(
      await app.invoke("apply_group_bookmarks_to_profile", {
        profileId: profileIds[1],
      }),
      false,
    );

    await app.invoke("delete_selected_profiles", { profileIds });
    await app.invoke("delete_profile_group", { groupId: group.id });
    for (const proxy of proxies) {
      await app.invoke("delete_stored_proxy", { proxyId: proxy.id });
    }
  });
});
