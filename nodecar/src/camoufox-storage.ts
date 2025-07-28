import fs from "node:fs";
import path from "node:path";
import tmp from "tmp";
import type { CamoufoxLaunchOptions } from "./camoufox-launcher.js";

export interface CamoufoxConfig {
  id: string;
  options: CamoufoxLaunchOptions;
  profilePath?: string;
  url?: string;
  port?: number;
  wsEndpoint?: string;
}

const STORAGE_DIR = path.join(tmp.tmpdir, "donutbrowser", "camoufox");

if (!fs.existsSync(STORAGE_DIR)) {
  fs.mkdirSync(STORAGE_DIR, { recursive: true });
}

/**
 * Save a Camoufox configuration to disk
 * @param config The Camoufox configuration to save
 */
export function saveCamoufoxConfig(config: CamoufoxConfig): void {
  const filePath = path.join(STORAGE_DIR, `${config.id}.json`);
  fs.writeFileSync(filePath, JSON.stringify(config, null, 2));
}

/**
 * Get a Camoufox configuration by ID
 * @param id The Camoufox ID
 * @returns The Camoufox configuration or null if not found
 */
export function getCamoufoxConfig(id: string): CamoufoxConfig | null {
  const filePath = path.join(STORAGE_DIR, `${id}.json`);

  if (!fs.existsSync(filePath)) {
    return null;
  }

  try {
    const content = fs.readFileSync(filePath, "utf-8");
    return JSON.parse(content) as CamoufoxConfig;
  } catch (error) {
    console.error(`Error reading Camoufox config ${id}:`, error);
    return null;
  }
}

/**
 * Delete a Camoufox configuration
 * @param id The Camoufox ID to delete
 * @returns True if deleted, false if not found
 */
export function deleteCamoufoxConfig(id: string): boolean {
  const filePath = path.join(STORAGE_DIR, `${id}.json`);

  if (!fs.existsSync(filePath)) {
    return false;
  }

  try {
    fs.unlinkSync(filePath);
    return true;
  } catch (error) {
    console.error(`Error deleting Camoufox config ${id}:`, error);
    return false;
  }
}

/**
 * List all saved Camoufox configurations
 * @returns Array of Camoufox configurations
 */
export function listCamoufoxConfigs(): CamoufoxConfig[] {
  if (!fs.existsSync(STORAGE_DIR)) {
    return [];
  }

  try {
    return fs
      .readdirSync(STORAGE_DIR)
      .filter((file) => file.endsWith(".json"))
      .map((file) => {
        try {
          const content = fs.readFileSync(
            path.join(STORAGE_DIR, file),
            "utf-8",
          );
          return JSON.parse(content) as CamoufoxConfig;
        } catch (error) {
          console.error(`Error reading Camoufox config ${file}:`, error);
          return null;
        }
      })
      .filter((config): config is CamoufoxConfig => config !== null);
  } catch (error) {
    console.error("Error listing Camoufox configs:", error);
    return [];
  }
}

/**
 * Update a Camoufox configuration
 * @param config The Camoufox configuration to update
 * @returns True if updated, false if not found
 */
export function updateCamoufoxConfig(config: CamoufoxConfig): boolean {
  const filePath = path.join(STORAGE_DIR, `${config.id}.json`);

  try {
    fs.readFileSync(filePath, "utf-8");
    fs.writeFileSync(filePath, JSON.stringify(config, null, 2));
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      console.error(
        `Config ${config.id} was deleted while the app was running`,
      );
      return false;
    }

    console.error(`Error updating Camoufox config ${config.id}:`, error);
    return false;
  }
}

/**
 * Check if a Camoufox server is running
 * @param port The port to check
 * @returns True if running, false otherwise
 */
export async function isServerRunning(port: number): Promise<boolean> {
  try {
    const response = await fetch(`http://localhost:${port}/json/version`, {
      method: "GET",
      signal: AbortSignal.timeout(1000),
    });
    return response.ok;
  } catch {
    return false;
  }
}

/**
 * Generate a unique ID for a Camoufox instance
 * @returns A unique ID string
 */
export function generateCamoufoxId(): string {
  return `camoufox_${Date.now()}_${Math.floor(Math.random() * 10000)}`;
}
