"use client";

import * as React from "react";
import { Area, AreaChart, ResponsiveContainer } from "recharts";
import { cn } from "@/lib/utils";
import type { BandwidthDataPoint } from "@/types";

interface BandwidthMiniChartProps {
  data: BandwidthDataPoint[];
  currentBandwidth?: number;
  onClick?: () => void;
  className?: string;
}

export function BandwidthMiniChart({
  data,
  currentBandwidth: externalBandwidth,
  onClick,
  className,
}: BandwidthMiniChartProps) {
  // Transform data for the chart - combine sent and received for total bandwidth
  const chartData = React.useMemo(() => {
    // Fill in missing seconds with zeros for smooth chart
    if (data.length === 0) {
      // Create 60 seconds of zero data for the past minute
      const now = Math.floor(Date.now() / 1000);
      return Array.from({ length: 60 }, (_, i) => ({
        time: now - (59 - i),
        bandwidth: 0,
      }));
    }

    const now = Math.floor(Date.now() / 1000);
    const result: { time: number; bandwidth: number }[] = [];

    // Get the last 60 seconds
    for (let i = 59; i >= 0; i--) {
      const targetTime = now - i;
      const point = data.find((d) => d.timestamp === targetTime);
      result.push({
        time: targetTime,
        bandwidth: point ? point.bytes_sent + point.bytes_received : 0,
      });
    }

    return result;
  }, [data]);

  // Find max value for scaling
  const _maxBandwidth = React.useMemo(() => {
    const max = Math.max(...chartData.map((d) => d.bandwidth), 1);
    return max;
  }, [chartData]);

  // Use external bandwidth if provided, otherwise calculate from last data point
  const currentBandwidth =
    externalBandwidth ?? chartData[chartData.length - 1]?.bandwidth ?? 0;

  // Format bytes to human readable
  const formatBytes = (bytes: number): string => {
    if (bytes === 0) return "0 B/s";
    if (bytes < 1024) return `${bytes} B/s`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB/s`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB/s`;
  };

  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "relative flex items-center gap-1.5 px-2 rounded cursor-pointer hover:bg-accent/50 transition-colors min-w-[130px] border-none bg-transparent",
        className,
      )}
    >
      <div className="flex-1 h-3">
        <ResponsiveContainer width="100%" height="100%">
          <AreaChart
            data={chartData}
            margin={{ top: 0, right: 0, bottom: 0, left: 0 }}
          >
            <defs>
              <linearGradient
                id="bandwidthGradient"
                x1="0"
                y1="0"
                x2="0"
                y2="1"
              >
                <stop
                  offset="0%"
                  stopColor="var(--chart-1)"
                  stopOpacity={0.6}
                />
                <stop
                  offset="100%"
                  stopColor="var(--chart-1)"
                  stopOpacity={0.1}
                />
              </linearGradient>
            </defs>
            <Area
              type="monotone"
              dataKey="bandwidth"
              stroke="var(--chart-1)"
              strokeWidth={1}
              fill="url(#bandwidthGradient)"
              isAnimationActive={false}
            />
          </AreaChart>
        </ResponsiveContainer>
      </div>
      <span className="text-xs text-muted-foreground whitespace-nowrap min-w-[60px] text-right">
        {formatBytes(currentBandwidth)}
      </span>
    </button>
  );
}
