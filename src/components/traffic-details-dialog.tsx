"use client";

import { invoke } from "@tauri-apps/api/core";
import * as React from "react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { TooltipContentProps } from "recharts/types/component/Tooltip";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  TooltipContent,
  TooltipTrigger,
  Tooltip as UITooltip,
} from "@/components/ui/tooltip";
import type { FilteredTrafficStats } from "@/types";

type TimePeriod =
  | "1m"
  | "5m"
  | "30m"
  | "1h"
  | "2h"
  | "4h"
  | "1d"
  | "7d"
  | "30d"
  | "all";

type DomainSortMetric = "total" | "sent" | "received" | "requests";

interface TrafficDetailsDialogProps {
  isOpen: boolean;
  onClose: () => void;
  profileId?: string;
  profileName?: string;
}

const formatBytes = (bytes: number): string => {
  if (bytes === 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
};

const formatBytesPerSecond = (bytes: number): string => {
  if (bytes === 0) return "0 B/s";
  if (bytes < 1024) return `${bytes} B/s`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB/s`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB/s`;
};

function getSecondsForPeriod(period: TimePeriod): number {
  switch (period) {
    case "1m":
      return 60;
    case "5m":
      return 300;
    case "30m":
      return 1800;
    case "1h":
      return 3600;
    case "2h":
      return 7200;
    case "4h":
      return 14400;
    case "1d":
      return 86400;
    case "7d":
      return 604800;
    case "30d":
      return 2592000;
    case "all":
      return 0; // 0 means all time
    default:
      return 300;
  }
}

const TruncatedDomain = React.memo<{ domain: string }>(({ domain }) => {
  const ref = React.useRef<HTMLSpanElement>(null);
  const [isTruncated, setIsTruncated] = React.useState(false);

  const checkTruncation = React.useCallback(() => {
    if (ref.current) {
      setIsTruncated(ref.current.scrollWidth > ref.current.clientWidth);
    }
  }, []);

  React.useLayoutEffect(() => {
    checkTruncation();
  });

  React.useEffect(() => {
    const resizeObserver = new ResizeObserver(checkTruncation);
    if (ref.current) {
      resizeObserver.observe(ref.current);
    }

    return () => {
      resizeObserver.disconnect();
    };
  }, [checkTruncation]);

  const content = (
    <span ref={ref} className="truncate max-w-[200px] block">
      {domain}
    </span>
  );

  if (!isTruncated) {
    return content;
  }

  return (
    <UITooltip>
      <TooltipTrigger asChild>{content}</TooltipTrigger>
      <TooltipContent>
        <p>{domain}</p>
      </TooltipContent>
    </UITooltip>
  );
});

TruncatedDomain.displayName = "TruncatedDomain";

export function TrafficDetailsDialog({
  isOpen,
  onClose,
  profileId,
  profileName,
}: TrafficDetailsDialogProps) {
  const [stats, setStats] = React.useState<FilteredTrafficStats | null>(null);
  const [timePeriod, setTimePeriod] = React.useState<TimePeriod>("5m");
  const [domainSortMetric, setDomainSortMetric] =
    React.useState<DomainSortMetric>("total");

  // Fetch stats periodically - now uses filtered API
  React.useEffect(() => {
    if (!isOpen || !profileId) return;

    const fetchStats = async () => {
      try {
        const seconds = getSecondsForPeriod(timePeriod);
        const filteredStats = await invoke<FilteredTrafficStats | null>(
          "get_traffic_stats_for_period",
          { profileId, seconds },
        );
        setStats(filteredStats);
      } catch (error) {
        console.error("Failed to fetch traffic stats:", error);
      }
    };

    void fetchStats();
    const interval = setInterval(fetchStats, 2000);

    return () => clearInterval(interval);
  }, [isOpen, profileId, timePeriod]);

  // Transform data for chart (already filtered by backend)
  const chartData = React.useMemo(() => {
    if (!stats?.bandwidth_history) return [];

    return stats.bandwidth_history.map((d) => ({
      time: d.timestamp,
      sent: d.bytes_sent,
      received: d.bytes_received,
      total: d.bytes_sent + d.bytes_received,
    }));
  }, [stats]);

  // Tooltip render function
  const renderTooltip = React.useCallback(
    (props: TooltipContentProps<number, string>) => {
      const { active, payload, label } = props;
      if (!active || !payload?.length) return null;

      const time = new Date((typeof label === "number" ? label : 0) * 1000);
      const formattedTime = time.toLocaleTimeString();

      return (
        <div className="bg-popover border rounded-lg px-3 py-2 shadow-lg">
          <p className="text-xs text-muted-foreground mb-1">{formattedTime}</p>
          {payload.map((entry) => (
            <p key={String(entry.dataKey)} className="text-sm">
              <span className="text-muted-foreground">
                {entry.dataKey === "sent" ? "↑ Sent: " : "↓ Received: "}
              </span>
              <span className="font-medium">
                {formatBytesPerSecond(
                  typeof entry.value === "number" ? entry.value : 0,
                )}
              </span>
            </p>
          ))}
        </div>
      );
    },
    [],
  );

  // Top domains sorted by selected metric
  const topDomains = React.useMemo(() => {
    if (!stats?.domains) return [];
    return Object.values(stats.domains)
      .sort((a, b) => {
        switch (domainSortMetric) {
          case "sent":
            return b.bytes_sent - a.bytes_sent;
          case "received":
            return b.bytes_received - a.bytes_received;
          case "total":
            return (
              b.bytes_sent +
              b.bytes_received -
              (a.bytes_sent + a.bytes_received)
            );
          case "requests":
            return b.request_count - a.request_count;
          default:
            return 0;
        }
      })
      .slice(0, 10);
  }, [stats, domainSortMetric]);

  return (
    <Dialog open={isOpen} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            Traffic Details
            {profileName && (
              <span className="text-muted-foreground font-normal ml-2">
                — {profileName}
              </span>
            )}
          </DialogTitle>
        </DialogHeader>

        <ScrollArea className="h-[60vh]">
          <div className="space-y-6 pr-4">
            {/* Chart with Period Selector */}
            <div>
              <div className="flex items-center justify-between mb-2">
                <h3 className="text-sm font-medium">Bandwidth Over Time</h3>
                <Select
                  value={timePeriod}
                  onValueChange={(v) => setTimePeriod(v as TimePeriod)}
                >
                  <SelectTrigger className="w-[120px] h-8">
                    <SelectValue placeholder="Time period" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="1m">Last 1 min</SelectItem>
                    <SelectItem value="5m">Last 5 min</SelectItem>
                    <SelectItem value="30m">Last 30 min</SelectItem>
                    <SelectItem value="1h">Last 1 hour</SelectItem>
                    <SelectItem value="2h">Last 2 hours</SelectItem>
                    <SelectItem value="4h">Last 4 hours</SelectItem>
                    <SelectItem value="1d">Last 1 day</SelectItem>
                    <SelectItem value="7d">Last 7 days</SelectItem>
                    <SelectItem value="30d">Last 30 days</SelectItem>
                    <SelectItem value="all">All time</SelectItem>
                  </SelectContent>
                </Select>
              </div>

              <div className="h-[200px] w-full">
                <ResponsiveContainer width="100%" height="100%">
                  <AreaChart
                    data={chartData}
                    margin={{ top: 10, right: 10, bottom: 0, left: 0 }}
                  >
                    <defs>
                      <linearGradient
                        id="sentGradient"
                        x1="0"
                        y1="0"
                        x2="0"
                        y2="1"
                      >
                        <stop
                          offset="0%"
                          stopColor="var(--chart-1)"
                          stopOpacity={0.5}
                        />
                        <stop
                          offset="100%"
                          stopColor="var(--chart-1)"
                          stopOpacity={0.1}
                        />
                      </linearGradient>
                      <linearGradient
                        id="receivedGradient"
                        x1="0"
                        y1="0"
                        x2="0"
                        y2="1"
                      >
                        <stop
                          offset="0%"
                          stopColor="var(--chart-2)"
                          stopOpacity={0.5}
                        />
                        <stop
                          offset="100%"
                          stopColor="var(--chart-2)"
                          stopOpacity={0.1}
                        />
                      </linearGradient>
                    </defs>
                    <CartesianGrid
                      strokeDasharray="3 3"
                      className="stroke-muted"
                    />
                    <XAxis
                      dataKey="time"
                      tickFormatter={(t) =>
                        new Date(t * 1000).toLocaleTimeString([], {
                          hour: "2-digit",
                          minute: "2-digit",
                        })
                      }
                      className="text-xs"
                      tick={{ fill: "var(--muted-foreground)" }}
                    />
                    <YAxis
                      tickFormatter={(v) => formatBytesPerSecond(v)}
                      className="text-xs"
                      tick={{ fill: "var(--muted-foreground)" }}
                      width={60}
                    />
                    <Tooltip content={renderTooltip} />
                    <Area
                      type="monotone"
                      dataKey="sent"
                      stackId="1"
                      stroke="var(--chart-1)"
                      fill="url(#sentGradient)"
                      strokeWidth={1.5}
                      isAnimationActive={false}
                    />
                    <Area
                      type="monotone"
                      dataKey="received"
                      stackId="1"
                      stroke="var(--chart-2)"
                      fill="url(#receivedGradient)"
                      strokeWidth={1.5}
                      isAnimationActive={false}
                    />
                  </AreaChart>
                </ResponsiveContainer>
              </div>

              <div className="flex items-center justify-center gap-6 mt-2">
                <div className="flex items-center gap-2">
                  <div
                    className="w-3 h-3 rounded"
                    style={{ backgroundColor: "var(--chart-1)" }}
                  />
                  <span className="text-xs text-muted-foreground">Sent</span>
                </div>
                <div className="flex items-center gap-2">
                  <div
                    className="w-3 h-3 rounded"
                    style={{ backgroundColor: "var(--chart-2)" }}
                  />
                  <span className="text-xs text-muted-foreground">
                    Received
                  </span>
                </div>
              </div>
            </div>

            {/* Period Stats - now uses backend-computed values */}
            <div className="grid grid-cols-3 gap-4">
              <div className="bg-muted/50 rounded-lg p-3">
                <p className="text-xs text-muted-foreground">
                  Sent ({timePeriod === "all" ? "total" : timePeriod})
                </p>
                <p className="text-lg font-semibold text-chart-1">
                  {formatBytes(stats?.period_bytes_sent || 0)}
                </p>
              </div>
              <div className="bg-muted/50 rounded-lg p-3">
                <p className="text-xs text-muted-foreground">
                  Received ({timePeriod === "all" ? "total" : timePeriod})
                </p>
                <p className="text-lg font-semibold text-chart-2">
                  {formatBytes(stats?.period_bytes_received || 0)}
                </p>
              </div>
              <div className="bg-muted/50 rounded-lg p-3">
                <p className="text-xs text-muted-foreground">
                  Requests ({timePeriod === "all" ? "total" : timePeriod})
                </p>
                <p className="text-lg font-semibold">
                  {(stats?.period_requests || 0).toLocaleString()}
                </p>
              </div>
            </div>

            {/* Total Stats (smaller, under period stats) */}
            <div className="flex items-center gap-6 text-sm text-muted-foreground border-t pt-4">
              <div>
                <span className="font-medium">All-time traffic:</span>{" "}
                {formatBytes(
                  (stats?.total_bytes_sent || 0) +
                    (stats?.total_bytes_received || 0),
                )}
              </div>
              <div>
                <span className="font-medium">All-time requests:</span>{" "}
                {stats?.total_requests?.toLocaleString() || 0}
              </div>
            </div>

            {/* Disclaimer about proxy/VPN traffic calculation */}
            <p className="text-xs text-muted-foreground italic">
              Note: If you are using a proxy, VPN, or similar service, your
              provider may calculate traffic differently due to encryption
              overhead and protocol differences.
            </p>

            {/* Top Domains */}
            {topDomains.length > 0 && (
              <div>
                <div className="flex items-center justify-between mb-2">
                  <h3 className="text-sm font-medium">
                    Top Domains by{" "}
                    {domainSortMetric === "total" && "Total Traffic"}
                    {domainSortMetric === "sent" && "Sent Traffic"}
                    {domainSortMetric === "received" && "Received Traffic"}
                    {domainSortMetric === "requests" && "Requests"} (
                    {timePeriod === "all" ? "all time" : timePeriod})
                  </h3>
                  <Select
                    value={domainSortMetric}
                    onValueChange={(v) =>
                      setDomainSortMetric(v as DomainSortMetric)
                    }
                  >
                    <SelectTrigger className="w-[140px] h-8">
                      <SelectValue placeholder="Sort by" />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="total">Total Traffic</SelectItem>
                      <SelectItem value="sent">Sent</SelectItem>
                      <SelectItem value="received">Received</SelectItem>
                      <SelectItem value="requests">Requests</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="border rounded-md">
                  <div className="grid grid-cols-[1fr_80px_80px_80px] gap-2 px-3 py-2 text-xs font-medium text-muted-foreground border-b bg-muted/30">
                    <span>Domain</span>
                    <span className="text-right">Requests</span>
                    <span className="text-right">Sent</span>
                    <span className="text-right">Received</span>
                  </div>
                  <div className="max-h-[180px] overflow-y-auto">
                    {topDomains.map((domain, index) => (
                      <div
                        key={domain.domain}
                        className="grid grid-cols-[1fr_80px_80px_80px] gap-2 px-3 py-2 text-sm border-b last:border-b-0 hover:bg-muted/30"
                      >
                        <div className="flex items-center gap-2 min-w-0">
                          <span className="text-xs text-muted-foreground w-4 shrink-0">
                            {index + 1}
                          </span>
                          <TruncatedDomain domain={domain.domain} />
                        </div>
                        <span className="text-right text-muted-foreground">
                          {domain.request_count.toLocaleString()}
                        </span>
                        <span className="text-right text-chart-1">
                          {formatBytes(domain.bytes_sent)}
                        </span>
                        <span className="text-right text-chart-2">
                          {formatBytes(domain.bytes_received)}
                        </span>
                      </div>
                    ))}
                  </div>
                </div>
              </div>
            )}

            {/* Unique IPs */}
            {stats?.unique_ips && stats.unique_ips.length > 0 && (
              <div>
                <h3 className="text-sm font-medium mb-2">
                  Unique IPs ({stats.unique_ips.length})
                </h3>
                <div className="border rounded-md p-3 max-h-[120px] overflow-y-auto">
                  <div className="flex flex-wrap gap-1.5">
                    {stats.unique_ips.map((ip) => (
                      <span
                        key={ip}
                        className="text-xs bg-muted px-2 py-1 rounded font-mono"
                      >
                        {ip}
                      </span>
                    ))}
                  </div>
                </div>
              </div>
            )}

            {/* No data state */}
            {!stats && (
              <div className="text-center py-8 text-muted-foreground">
                <p>No traffic data available for this profile.</p>
                <p className="text-sm mt-1">
                  Traffic data will appear after you launch the profile.
                </p>
              </div>
            )}
          </div>
        </ScrollArea>
      </DialogContent>
    </Dialog>
  );
}
