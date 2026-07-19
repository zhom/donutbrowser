import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { appFromEnvironment } from "../lib/app.mjs";
import { extensionZipBase64, wireGuardFixture } from "../lib/fixtures.mjs";

const syncUrl = process.env.DONUT_E2E_SYNC_URL;
const syncToken = process.env.DONUT_E2E_SYNC_TOKEN;

async function syncRequest(endpoint, body) {
  const response = await fetch(`${syncUrl}/v1/objects/${endpoint}`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${syncToken}`,
      "content-type": "application/json",
    },
    body: JSON.stringify(body),
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(
      `Sync ${endpoint} failed with HTTP ${response.status}: ${text}`,
    );
  }
  return text ? JSON.parse(text) : null;
}

async function listRemote(prefix = "") {
  const result = await syncRequest("list", {
    prefix,
    maxKeys: 1000,
    continuationToken: null,
  });
  return result.objects;
}

async function downloadRemote(key) {
  const presigned = await syncRequest("presign-download", {
    key,
    expiresIn: 300,
  });
  const response = await fetch(presigned.url);
  assert.equal(response.status, 200, `Could not download remote object ${key}`);
  return Buffer.from(await response.arrayBuffer());
}

async function configureSync(app) {
  const saved = await app.invoke("save_sync_settings", {
    syncServerUrl: syncUrl,
    syncToken,
  });
  assert.equal(saved.sync_server_url, syncUrl);
  assert.equal(saved.sync_token, syncToken);
  assert.deepEqual(await app.invoke("get_sync_settings"), saved);
  await app.invoke("restart_sync_service");
  await new Promise((resolve) => setTimeout(resolve, 750));
}

async function createProfile(app, name) {
  return app.invoke("create_browser_profile_new", {
    name,
    browserStr: "wayfern",
    version: "150.0.7871.100",
    releaseType: "stable",
    proxyId: null,
    vpnId: null,
    // Keep sync tests deterministic and network-free; browser.test.mjs covers
    // generation through the real Wayfern binary.
    wayfernConfig: { fingerprint: "{}" },
    groupId: null,
    ephemeral: false,
    dnsBlocklist: null,
    launchHook: null,
  });
}

async function waitFor(app, callback, description, timeoutMs = 45_000) {
  return app.waitFor(callback, { description, timeoutMs, intervalMs: 250 });
}

test("two real app devices reconcile profile files and every config entity with last-write-wins", async () => {
  assert.ok(syncUrl && syncToken, "Sync infrastructure was not started");
  const deviceA = appFromEnvironment("sync-regular-a");
  const deviceB = appFromEnvironment("sync-regular-b");
  try {
    await Promise.all([deviceA.start(), deviceB.start()]);
    await Promise.all([configureSync(deviceA), configureSync(deviceB)]);

    const group = await deviceA.invoke("create_profile_group", {
      name: "Synced Group A",
    });
    const proxy = await deviceA.invoke("create_stored_proxy", {
      name: "Synced Proxy A",
      proxySettings: {
        proxy_type: "http",
        host: "127.0.0.1",
        port: 8089,
        username: null,
        password: null,
      },
    });
    const vpn = await deviceA.invoke("create_vpn_config_manual", {
      name: "Synced VPN A",
      vpnType: "WireGuard",
      configData: wireGuardFixture(),
    });
    const extension = await deviceA.invoke("add_extension", {
      name: "Synced Extension A",
      fileName: "synced-fixture.zip",
      fileData: [...Buffer.from(extensionZipBase64(), "base64")],
    });
    const extensionGroup = await deviceA.invoke("create_extension_group", {
      name: "Synced Extension Group A",
    });
    await deviceA.invoke("add_extension_to_group", {
      groupId: extensionGroup.id,
      extensionId: extension.id,
    });

    await Promise.all([
      deviceA.invoke("set_group_sync_enabled", {
        groupId: group.id,
        enabled: true,
      }),
      deviceA.invoke("set_proxy_sync_enabled", {
        proxyId: proxy.id,
        enabled: true,
      }),
      deviceA.invoke("set_vpn_sync_enabled", { vpnId: vpn.id, enabled: true }),
      deviceA.invoke("set_extension_sync_enabled", {
        extensionId: extension.id,
        enabled: true,
      }),
      deviceA.invoke("set_extension_group_sync_enabled", {
        extensionGroupId: extensionGroup.id,
        enabled: true,
      }),
    ]);

    const profile = await createProfile(deviceA, "Synced Profile A");
    const profileData = path.join(
      deviceA.dataRoot,
      "data",
      "profiles",
      profile.id,
      "profile",
      "Default",
    );
    await mkdir(profileData, { recursive: true });
    await writeFile(
      path.join(profileData, "Preferences"),
      JSON.stringify({ donutE2E: "regular-profile-payload" }),
    );
    await deviceA.invoke("update_profile_tags", {
      profileId: profile.id,
      tags: ["sync", "device-a"],
    });
    await deviceA.invoke("update_profile_note", {
      profileId: profile.id,
      note: "regular sync metadata",
    });
    await deviceA.invoke("set_profile_sync_mode", {
      profileId: profile.id,
      syncMode: "Regular",
    });
    await deviceA.invoke("request_profile_sync", { profileId: profile.id });
    assert.equal(
      await deviceA.invoke("cancel_profile_sync", {
        profileId: "not-running-sync",
      }),
      false,
    );

    await waitFor(
      deviceA,
      async () => {
        const keys = (await listRemote("")).map((object) => object.key);
        return [
          `groups/${group.id}.json`,
          `proxies/${proxy.id}.json`,
          `vpns/${vpn.id}.json`,
          `extensions/${extension.id}.json`,
          `extension_groups/${extensionGroup.id}.json`,
          `profiles/${profile.id}/manifest.json`,
          `profiles/${profile.id}/files/profile/Default/Preferences`,
        ].every((key) => keys.includes(key));
      },
      "all regular entities uploaded",
    );

    await deviceB.invoke("restart_sync_service");
    await waitFor(
      deviceB,
      async () => {
        const [profiles, groups, proxies, vpns, extensions, extensionGroups] =
          await Promise.all([
            deviceB.invoke("list_browser_profiles"),
            deviceB.invoke("get_profile_groups"),
            deviceB.invoke("get_stored_proxies"),
            deviceB.invoke("list_vpn_configs"),
            deviceB.invoke("list_extensions"),
            deviceB.invoke("list_extension_groups"),
          ]);
        return (
          profiles.some((item) => item.id === profile.id) &&
          groups.some((item) => item.id === group.id) &&
          proxies.some((item) => item.id === proxy.id) &&
          vpns.some((item) => item.id === vpn.id) &&
          extensions.some((item) => item.id === extension.id) &&
          extensionGroups.some((item) => item.id === extensionGroup.id)
        );
      },
      "device B receives every entity",
    );
    const downloadedPreferences = path.join(
      deviceB.dataRoot,
      "data",
      "profiles",
      profile.id,
      "profile",
      "Default",
      "Preferences",
    );
    await waitFor(
      deviceB,
      async () =>
        (
          await readFile(downloadedPreferences, "utf8").catch(() => "")
        ).includes("regular-profile-payload"),
      "device B receives profile browser files",
    );

    // updated_at has one-second resolution. Make the device-B edits
    // unambiguously newer, then verify last-write-wins in both directions.
    await new Promise((resolve) => setTimeout(resolve, 1_100));
    await deviceB.invoke("update_stored_proxy", {
      proxyId: proxy.id,
      name: "Synced Proxy B Wins",
      proxySettings: null,
    });
    await deviceB.invoke("rename_profile", {
      profileId: profile.id,
      newName: "Synced Profile B Wins",
    });
    await deviceB.invoke("request_profile_sync", { profileId: profile.id });
    await deviceA.invoke("restart_sync_service");
    await waitFor(
      deviceA,
      async () => {
        const proxies = await deviceA.invoke("get_stored_proxies");
        const profiles = await deviceA.invoke("list_browser_profiles");
        return (
          proxies.find((item) => item.id === proxy.id)?.name ===
            "Synced Proxy B Wins" &&
          profiles.find((item) => item.id === profile.id)?.name ===
            "Synced Profile B Wins"
        );
      },
      "newer device-B edits win on device A",
    );

    assert.equal(
      await deviceA.invoke("is_proxy_in_use_by_synced_profile", {
        proxyId: proxy.id,
      }),
      false,
    );
    assert.equal(
      await deviceA.invoke("is_group_in_use_by_synced_profile", {
        groupId: group.id,
      }),
      false,
    );
    assert.equal(
      await deviceA.invoke("is_vpn_in_use_by_synced_profile", {
        vpnId: vpn.id,
      }),
      false,
    );
    const counts = await deviceA.invoke("get_unsynced_entity_counts");
    assert.equal(typeof counts.proxies, "number");
    await deviceA.invoke("enable_sync_for_all_entities");

    await Promise.all([
      deviceB.invoke("delete_extension_group", {
        groupId: extensionGroup.id,
      }),
      deviceB.invoke("delete_extension", { extensionId: extension.id }),
      deviceB.invoke("delete_vpn_config", { vpnId: vpn.id }),
      deviceB.invoke("delete_profile_group", { groupId: group.id }),
      deviceB.invoke("delete_stored_proxy", { proxyId: proxy.id }),
      deviceB.invoke("delete_profile", { profileId: profile.id }),
    ]);
    await waitFor(
      deviceB,
      async () => {
        const keys = (await listRemote("")).map((object) => object.key);
        return [
          `tombstones/groups/${group.id}.json`,
          `tombstones/proxies/${proxy.id}.json`,
          `tombstones/vpns/${vpn.id}.json`,
          `tombstones/extensions/${extension.id}.json`,
          `tombstones/extension_groups/${extensionGroup.id}.json`,
          `tombstones/profiles/${profile.id}.json`,
        ].every((key) => keys.includes(key));
      },
      "deletions create every remote tombstone",
    );
    await waitFor(
      deviceA,
      async () => {
        const [profiles, groups, proxies, vpns, extensions, extensionGroups] =
          await Promise.all([
            deviceA.invoke("list_browser_profiles"),
            deviceA.invoke("get_profile_groups"),
            deviceA.invoke("get_stored_proxies"),
            deviceA.invoke("list_vpn_configs"),
            deviceA.invoke("list_extensions"),
            deviceA.invoke("list_extension_groups"),
          ]);
        return (
          !profiles.some((item) => item.id === profile.id) &&
          !groups.some((item) => item.id === group.id) &&
          !proxies.some((item) => item.id === proxy.id) &&
          !vpns.some((item) => item.id === vpn.id) &&
          !extensions.some((item) => item.id === extension.id) &&
          !extensionGroups.some((item) => item.id === extensionGroup.id)
        );
      },
      "remote tombstones delete every entity from device A",
    );
  } catch (error) {
    await Promise.all([deviceA.capture("failure"), deviceB.capture("failure")]);
    throw error;
  } finally {
    await Promise.all([deviceA.close(), deviceB.close()]);
  }
});

test("global config sealing and encrypted profile sync reject a wrong password, round-trip with the right one, and roll over", async () => {
  const source = appFromEnvironment("sync-encrypted-source");
  const receiver = appFromEnvironment("sync-encrypted-receiver");
  const rolloverReceiver = appFromEnvironment(
    "sync-encrypted-rollover-receiver",
  );
  try {
    await Promise.all([source.start(), receiver.start()]);
    await Promise.all([configureSync(source), configureSync(receiver)]);
    await source.invoke("set_e2e_password", {
      password: "shared encryption password",
    });
    await receiver.invoke("set_e2e_password", {
      password: "intentionally wrong password",
    });
    assert.equal(await source.invoke("check_has_e2e_password"), true);
    assert.equal(
      await source.invoke("verify_e2e_password", {
        password: "shared encryption password",
      }),
      true,
    );
    assert.equal(
      await source.invoke("verify_e2e_password", { password: "wrong" }),
      false,
    );

    const sealedProxy = await source.invoke("create_stored_proxy", {
      name: "SECRET-CONFIG-MARKER",
      proxySettings: {
        proxy_type: "http",
        host: "secret-proxy.invalid",
        port: 8443,
        username: "secret-user",
        password: "secret-password",
      },
    });
    await source.invoke("set_proxy_sync_enabled", {
      proxyId: sealedProxy.id,
      enabled: true,
    });

    const encryptedProfile = await createProfile(source, "Encrypted Profile");
    const encryptedData = path.join(
      source.dataRoot,
      "data",
      "profiles",
      encryptedProfile.id,
      "profile",
    );
    await mkdir(encryptedData, { recursive: true });
    await writeFile(
      path.join(encryptedData, "Local State"),
      "SECRET-PROFILE-MARKER that must never appear remotely",
    );
    await source.invoke("set_profile_sync_mode", {
      profileId: encryptedProfile.id,
      syncMode: "Encrypted",
    });
    await source.invoke("request_profile_sync", {
      profileId: encryptedProfile.id,
    });

    const proxyKey = `proxies/${sealedProxy.id}.json`;
    const profileMetadataKey = `profiles/${encryptedProfile.id}/metadata.json`;
    const profileFileKey = `profiles/${encryptedProfile.id}/files/profile/Local State`;
    await waitFor(
      source,
      async () => {
        const keys = (await listRemote("")).map((object) => object.key);
        return (
          keys.includes(proxyKey) &&
          keys.includes(profileMetadataKey) &&
          keys.includes(profileFileKey)
        );
      },
      "sealed config and encrypted profile uploaded",
    );
    const sealedBefore = await downloadRemote(proxyKey);
    const metadataBefore = await downloadRemote(profileMetadataKey);
    const encryptedFile = await downloadRemote(profileFileKey);
    assert.equal(
      sealedBefore.includes(Buffer.from("SECRET-CONFIG-MARKER")),
      false,
    );
    assert.equal(sealedBefore.includes(Buffer.from("secret-password")), false);
    assert.equal(
      encryptedFile.includes(Buffer.from("SECRET-PROFILE-MARKER")),
      false,
    );
    const envelope = JSON.parse(sealedBefore.toString("utf8"));
    assert.equal(envelope.v, 1);
    assert.ok(envelope.salt && envelope.ct);

    await receiver.invoke("restart_sync_service");
    await new Promise((resolve) => setTimeout(resolve, 2_000));
    assert.equal(
      (await receiver.invoke("get_stored_proxies")).some(
        (item) => item.id === sealedProxy.id,
      ),
      false,
      "wrong password must not materialize sealed config",
    );
    assert.equal(
      (await receiver.invoke("list_browser_profiles")).some(
        (item) => item.id === encryptedProfile.id,
      ),
      false,
      "wrong password must not materialize encrypted profiles",
    );

    await receiver.invoke("set_e2e_password", {
      password: "shared encryption password",
    });
    await receiver.invoke("restart_sync_service");
    await waitFor(
      receiver,
      async () =>
        (await receiver.invoke("get_stored_proxies")).some(
          (item) =>
            item.id === sealedProxy.id && item.name === "SECRET-CONFIG-MARKER",
        ) &&
        (await receiver.invoke("list_browser_profiles")).some(
          (item) => item.id === encryptedProfile.id,
        ),
      "correct password decrypts config and profile metadata",
    );
    const receiverFile = path.join(
      receiver.dataRoot,
      "data",
      "profiles",
      encryptedProfile.id,
      "profile",
      "Local State",
    );
    await waitFor(
      receiver,
      async () =>
        (await readFile(receiverFile, "utf8").catch(() => "")).includes(
          "SECRET-PROFILE-MARKER",
        ),
      "correct password decrypts profile browser file",
    );

    await source.invoke("set_e2e_password", {
      password: "rolled encryption password",
    });
    await source.invoke("rollover_encryption_for_all_entities");
    await waitFor(
      source,
      async () => {
        const [proxy, metadata] = await Promise.all([
          downloadRemote(proxyKey),
          downloadRemote(profileMetadataKey),
        ]);
        return !proxy.equals(sealedBefore) && !metadata.equals(metadataBefore);
      },
      "password rollover rewrites sealed config and profile metadata",
    );
    const sealedAfter = await downloadRemote(proxyKey);
    assert.equal(
      sealedAfter.includes(Buffer.from("SECRET-CONFIG-MARKER")),
      false,
    );
    await receiver.invoke("set_e2e_password", {
      password: "rolled encryption password",
    });
    await receiver.invoke("restart_sync_service");
    await waitFor(
      receiver,
      async () =>
        (await receiver.invoke("get_stored_proxies")).some(
          (item) =>
            item.id === sealedProxy.id && item.name === "SECRET-CONFIG-MARKER",
        ),
      "receiver accepts rolled password",
    );

    await rolloverReceiver.start();
    await rolloverReceiver.invoke("set_e2e_password", {
      password: "rolled encryption password",
    });
    await configureSync(rolloverReceiver);
    await waitFor(
      rolloverReceiver,
      async () =>
        (await rolloverReceiver.invoke("get_stored_proxies")).some(
          (item) =>
            item.id === sealedProxy.id && item.name === "SECRET-CONFIG-MARKER",
        ) &&
        (await rolloverReceiver.invoke("list_browser_profiles")).some(
          (item) => item.id === encryptedProfile.id,
        ),
      "fresh receiver decrypts rolled config and profile metadata",
    );
    const rolloverFile = path.join(
      rolloverReceiver.dataRoot,
      "data",
      "profiles",
      encryptedProfile.id,
      "profile",
      "Local State",
    );
    await waitFor(
      rolloverReceiver,
      async () =>
        (await readFile(rolloverFile, "utf8").catch(() => "")).includes(
          "SECRET-PROFILE-MARKER",
        ),
      "fresh receiver decrypts rolled profile browser file",
    );

    await source.invoke("set_profile_sync_mode", {
      profileId: encryptedProfile.id,
      syncMode: "Disabled",
    });
    await source.invoke("delete_e2e_password");
    assert.equal(await source.invoke("check_has_e2e_password"), false);
    const missingPassword = await source.invokeError("verify_e2e_password", {
      password: "rolled encryption password",
    });
    assert.match(missingPassword, /NO_E2E_PASSWORD_SET/);
  } catch (error) {
    await Promise.all([
      source.capture("failure"),
      receiver.capture("failure"),
      rolloverReceiver.capture("failure"),
    ]);
    throw error;
  } finally {
    await Promise.all([
      source.close(),
      receiver.close(),
      rolloverReceiver.close(),
    ]);
  }
});
