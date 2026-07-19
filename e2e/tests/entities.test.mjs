import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { withApp } from "../lib/app.mjs";
import { extensionZipBase64, wireGuardFixture } from "../lib/fixtures.mjs";

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
  await withApp("entities-core", async (app) => {
    await app.invoke("complete_onboarding");
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

    const exported = JSON.parse(
      await app.invoke("export_proxies", { format: "json" }),
    );
    assert.equal(exported.proxies.length, 2);
    assert.ok(exported.proxies.some((item) => item.name === "Updated Proxy"));
    assert.ok(exported.proxies.some((item) => item.name === "Parsed Proxy 1"));
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
      JSON.stringify({ profile: { name: "Imported fixture" } }),
    );
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
      wayfernConfig: null,
    });
    assert.equal(importBatch.imported_count + importBatch.failed_count, 1);
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
      profileIds: [profile.id, clone.id],
    });
    assert.deepEqual(await app.invoke("list_browser_profiles"), []);
    await app.invoke("delete_profile_group", { groupId: group.id });
    await app.invoke("delete_stored_proxy", { proxyId: proxy.id });
    for (const importedProxy of (await app.invoke("get_stored_proxies")).filter(
      (item) =>
        item.name === "Imported Proxy" || item.name.startsWith("Parsed Proxy"),
    )) {
      await app.invoke("delete_stored_proxy", { proxyId: importedProxy.id });
    }
  });
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
    assert.match(unknownVpnError, /not found|Failed to start VPN worker/i);
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
    const textExport = await app.invoke("export_custom_dns_rules", {
      format: "txt",
    });
    assert.match(textExport, /ads\.example\.com/);
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
