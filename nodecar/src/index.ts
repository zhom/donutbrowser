import { program } from "commander";
import {
  startProxyProcess,
  stopAllProxyProcesses,
  stopProxyProcess,
} from "./proxy-runner";
import { listProxyConfigs } from "./proxy-storage";
import { runProxyWorker } from "./proxy-worker";

// Command for proxy management
program
  .command("proxy")
  .argument("<action>", "start, stop, or list proxies")
  .option("--host <host>", "upstream proxy host")
  .option("--proxy-port <port>", "upstream proxy port", Number.parseInt)
  .option("--type <type>", "proxy type (http, https, socks4, socks5)")
  .option("--username <username>", "proxy username")
  .option("--password <password>", "proxy password")
  .option(
    "-p, --port <number>",
    "local port to use (random if not specified)",
    Number.parseInt,
  )
  .option("--ignore-certificate", "ignore certificate errors for HTTPS proxies")
  .option("--id <id>", "proxy ID for stop command")
  .option(
    "-u, --upstream <url>",
    "upstream proxy URL (protocol://[username:password@]host:port)",
  )
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
        upstream?: string;
      },
    ) => {
      if (action === "start") {
        let upstreamUrl: string;

        // Build upstream URL from individual components if provided
        if (options.host && options.proxyPort && options.type) {
          const protocol =
            options.type === "socks4" || options.type === "socks5"
              ? options.type
              : "http";
          const auth =
            options.username && options.password
              ? `${encodeURIComponent(options.username)}:${encodeURIComponent(
                  options.password,
                )}@`
              : "";
          upstreamUrl = `${protocol}://${auth}${options.host}:${options.proxyPort}`;
        } else if (options.upstream) {
          upstreamUrl = options.upstream;
        } else {
          console.error(
            "Error: Either --upstream URL or --host, --proxy-port, and --type are required",
          );
          console.log(
            "Example: proxy start --host datacenter.proxyempire.io --proxy-port 9000 --type http --username user --password pass",
          );
          process.exit(1);
          return;
        }

        try {
          const config = await startProxyProcess(upstreamUrl, {
            port: options.port,
            ignoreProxyCertificate: options.ignoreCertificate,
          });

          // Output the configuration as JSON for the Rust side to parse
          console.log(
            JSON.stringify({
              id: config.id,
              localPort: config.localPort,
              localUrl: config.localUrl,
              upstreamUrl: config.upstreamUrl,
            }),
          );

          // Exit successfully to allow the process to detach
          process.exit(0);
        } catch (error: unknown) {
          console.error(
            `Failed to start proxy: ${
              error instanceof Error ? error.message : JSON.stringify(error)
            }`,
          );
          process.exit(1);
        }
      } else if (action === "stop") {
        if (options.id) {
          const stopped = await stopProxyProcess(options.id);
          console.log(JSON.stringify({ success: stopped }));
        } else if (options.upstream) {
          // Find proxies with this upstream URL
          const configs = listProxyConfigs().filter(
            (config) => config.upstreamUrl === options.upstream,
          );

          if (configs.length === 0) {
            console.error(`No proxies found for ${options.upstream}`);
            process.exit(1);
            return;
          }

          for (const config of configs) {
            const stopped = await stopProxyProcess(config.id);
            console.log(JSON.stringify({ success: stopped }));
          }
        } else {
          await stopAllProxyProcesses();
          console.log(JSON.stringify({ success: true }));
        }
        process.exit(0);
      } else if (action === "list") {
        const configs = listProxyConfigs();
        console.log(JSON.stringify(configs));
        process.exit(0);
      } else {
        console.error("Invalid action. Use 'start', 'stop', or 'list'");
        process.exit(1);
      }
    },
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
