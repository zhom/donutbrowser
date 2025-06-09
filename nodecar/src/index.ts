import { program } from "commander";
import {
  startProxyProcess,
  stopProxyProcess,
  stopAllProxyProcesses,
} from "./proxy-runner";
import { listProxyConfigs } from "./proxy-storage";
import { runProxyWorker } from "./proxy-worker";

// Command for proxy management
program
  .command("proxy")
  .argument("<action>", "start, stop, or list proxies")
  .option("-h, --host <host>", "upstream proxy host")
  .option("-P, --proxy-port <port>", "upstream proxy port", Number.parseInt)
  .option(
    "-t, --type <type>",
    "upstream proxy type (http, https, socks4, socks5)",
    "http"
  )
  .option("-u, --username <username>", "upstream proxy username")
  .option("-w, --password <password>", "upstream proxy password")
  .option(
    "-p, --port <number>",
    "local port to use (random if not specified)",
    Number.parseInt
  )
  .option("--ignore-certificate", "ignore certificate errors for HTTPS proxies")
  .option("--id <id>", "proxy ID for stop command")
  .description("manage proxy servers")
  .action(
    async (
      action: string,
      options: {
        host?: string;
        proxyPort?: number;
        type?: string;
        username?: string;
        password?: string;
        port?: number;
        ignoreCertificate?: boolean;
        id?: string;
      }
    ) => {
      if (action === "start") {
        if (!options.host || !options.proxyPort) {
          console.error("Error: Upstream proxy host and port are required");
          console.log(
            "Example: proxy start -h proxy.example.com -P 8080 -t http -u username -w password"
          );
          return;
        }

        try {
          // Construct the upstream URL with credentials if provided
          let upstreamProxyUrl: string;
          if (options.username && options.password) {
            upstreamProxyUrl = `${options.type}://${options.username}:${options.password}@${options.host}:${options.proxyPort}`;
          } else {
            upstreamProxyUrl = `${options.type}://${options.host}:${options.proxyPort}`;
          }

          const config = await startProxyProcess(upstreamProxyUrl, {
            port: options.port,
            ignoreProxyCertificate: options.ignoreCertificate,
          });
          console.log(JSON.stringify(config));
        } catch (error: unknown) {
          console.error(`Failed to start proxy: ${JSON.stringify(error)}`);
        }
      } else if (action === "stop") {
        if (options.id) {
          const stopped = await stopProxyProcess(options.id);
          console.log(`{
            "success": ${stopped}}`);
        } else if (options.host && options.proxyPort && options.type) {
          // Find proxies with matching upstream details
          const configs = listProxyConfigs().filter((config) => {
            try {
              const url = new URL(config.upstreamUrl);
              return (
                url.hostname === options.host &&
                Number.parseInt(url.port) === options.proxyPort &&
                url.protocol.replace(":", "") === options.type
              );
            } catch {
              return false;
            }
          });

          if (configs.length === 0) {
            console.error(
              `No proxies found for ${options.host}:${options.proxyPort}`
            );
            return;
          }

          for (const config of configs) {
            const stopped = await stopProxyProcess(config.id);
            console.log(`{
            "success": ${stopped}}`);
          }
        } else {
          await stopAllProxyProcesses();
          console.log(`{
            "success": true}`);
        }
      } else if (action === "list") {
        const configs = listProxyConfigs();
        console.log(JSON.stringify(configs));
      } else {
        console.error("Invalid action. Use 'start', 'stop', or 'list'");
      }
    }
  );

// Command for proxy worker (internal use)
program
  .command("proxy-worker")
  .argument("<action>", "start a proxy worker")
  .requiredOption("--id <id>", "proxy configuration ID")
  .description("run a proxy worker process")
  .action(async (action: string, options: { id: string }) => {
    if (action === "start") {
      await runProxyWorker(options.id);
    } else {
      console.error("Invalid action for proxy-worker. Use 'start'");
      process.exit(1);
    }
  });

program.parse();
