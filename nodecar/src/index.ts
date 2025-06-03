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
  .option(
    "-u, --upstream <url>",
    "upstream proxy URL (protocol://[username:password@]host:port)"
  )
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
        upstream?: string;
        port?: number;
        ignoreCertificate?: boolean;
        id?: string;
      }
    ) => {
      if (action === "start") {
        if (!options.upstream) {
          console.error("Error: Upstream proxy URL is required");
          console.log(
            "Example: proxy start -u http://username:password@proxy.example.com:8080"
          );
          return;
        }

        try {
          const config = await startProxyProcess(options.upstream, {
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
        } else if (options.upstream) {
          // Find proxies with this upstream URL
          const configs = listProxyConfigs().filter(
            (config) => config.upstreamUrl === options.upstream
          );

          if (configs.length === 0) {
            console.error(`No proxies found for ${options.upstream}`);
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
