import type { IconType } from "react-icons";
import { LuAppWindow, LuCodeXml, LuPlug, LuTerminal } from "react-icons/lu";
import {
  SiClaude,
  SiCursor,
  SiWindsurf,
  SiZedindustries,
} from "react-icons/si";
import { VscVscode } from "react-icons/vsc";
import type { AgentCategory } from "@/lib/mcp";

const BRAND_MARKS: Record<string, IconType> = {
  "claude-desktop": SiClaude,
  "claude-code": SiClaude,
  cursor: SiCursor,
  vscode: VscVscode,
  windsurf: SiWindsurf,
  zed: SiZedindustries,
};

/** The client's own mark when it has one, otherwise a mark for its kind. */
export function AgentIcon({
  category,
  id,
}: {
  category: AgentCategory;
  id: string;
}) {
  const className = "size-5 shrink-0 text-muted-foreground";
  const Mark = BRAND_MARKS[id];
  if (Mark) {
    return <Mark aria-hidden="true" className={className} />;
  }
  switch (category) {
    case "desktop-app":
      return <LuAppWindow aria-hidden="true" className={className} />;
    case "editor":
      return <LuCodeXml aria-hidden="true" className={className} />;
    case "editor-ext":
      return <LuPlug aria-hidden="true" className={className} />;
    case "cli":
      return <LuTerminal aria-hidden="true" className={className} />;
  }
}
