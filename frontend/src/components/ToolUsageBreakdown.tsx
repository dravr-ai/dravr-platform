// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useQuery } from '@tanstack/react-query';
import { dashboardApi } from '../services/api';
import { Doughnut, Bar } from 'react-chartjs-2';
import type { ToolUsageBreakdown } from '../types/api';
import { QUERY_KEYS } from '../constants/queryKeys';

interface ToolUsageBreakdownProps {
  apiKeyId?: string;
  timeRange?: string;
}

export default function ToolUsageBreakdownComponent({
  apiKeyId,
  timeRange = '7d'
}: ToolUsageBreakdownProps) {
  const { data: toolUsage, isLoading } = useQuery<ToolUsageBreakdown[]>({
    queryKey: QUERY_KEYS.dashboard.toolUsage(apiKeyId, timeRange),
    queryFn: () => dashboardApi.getToolUsageBreakdown(apiKeyId, timeRange),
  });

  if (isLoading) {
    return (
      <div className="flex justify-center py-8">
        <div className="pierre-spinner w-8 h-8"></div>
      </div>
    );
  }

  if (!toolUsage || toolUsage.length === 0) {
    return (
      <div className="card-dark">
        <div className="text-center py-8 text-on-surface-variant">
          <div className="text-4xl mb-4">🔧</div>
          <p className="text-lg mb-2 text-on-surface">No tool usage data</p>
          <p>Start making API calls to see tool usage breakdown</p>
        </div>
      </div>
    );
  }

  // Prepare chart data - Pierre brand colors
  const colors = [
    '#00241a', // pierre-violet
    '#0d3b2e', // pierre-cyan
    '#3c6658', // pierre-activity
    '#8f6a2e', // pierre-nutrition
    '#FF6B6B', // pierre-red
    '#0d3b2e', // cyan
    '#FB923C', // orange
    '#A78BFA', // light violet
  ];

  const doughnutData = {
    labels: toolUsage.map(tool => tool.tool_name.replace('_', ' ').replace(/\b\w/g, l => l.toUpperCase())),
    datasets: [
      {
        data: toolUsage.map(tool => tool.request_count),
        backgroundColor: colors.slice(0, toolUsage.length),
        borderColor: colors.slice(0, toolUsage.length),
        borderWidth: 2,
      },
    ],
  };

  const barData = {
    labels: toolUsage.map(tool => tool.tool_name.replace('_', ' ').replace(/\b\w/g, l => l.toUpperCase())),
    datasets: [
      {
        label: 'Avg Response Time (ms)',
        data: toolUsage.map(tool => tool.average_response_time),
        backgroundColor: 'rgba(0, 36, 26, 0.6)',
        borderColor: 'rgb(139, 92, 246)',
        borderWidth: 1,
      },
    ],
  };

  const doughnutOptions = {
    responsive: true,
    maintainAspectRatio: false,
    plugins: {
      legend: {
        position: 'bottom' as const,
        labels: {
          color: '#a1a1aa', // zinc-400
        },
      },
      tooltip: {
        callbacks: {
          label: function(context: { label: string; dataIndex: number }) {
            const tool = toolUsage[context.dataIndex];
            const totalRequests = toolUsage.reduce((sum, t) => sum + t.request_count, 0);
            const percentage = totalRequests > 0 ? (tool.request_count / totalRequests) * 100 : 0;
            return `${context.label}: ${tool.request_count} requests (${percentage.toFixed(1)}%)`;
          }
        }
      }
    },
  };

  const barOptions = {
    responsive: true,
    maintainAspectRatio: false,
    plugins: {
      legend: {
        display: false,
      },
    },
    scales: {
      y: {
        beginAtZero: true,
        title: {
          display: true,
          text: 'Response Time (ms)',
          color: '#a1a1aa', // zinc-400
        },
        ticks: {
          color: '#a1a1aa', // zinc-400
        },
        grid: {
          color: 'rgba(255, 255, 255, 0.1)',
        },
      },
      x: {
        ticks: {
          maxRotation: 45,
          color: '#a1a1aa', // zinc-400
        },
        grid: {
          color: 'rgba(255, 255, 255, 0.1)',
        },
      }
    },
  };

  const formatToolName = (toolName: string) => {
    return toolName.replace('_', ' ').replace(/\b\w/g, l => l.toUpperCase());
  };

  return (
    <div className="space-y-6">
      {/* Tool Usage Distribution */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        <div className="card-dark">
          <h3 className="text-lg font-medium mb-4 text-on-surface">Request Distribution</h3>
          <div style={{ height: '300px' }}>
            <Doughnut data={doughnutData} options={doughnutOptions} />
          </div>
        </div>

        <div className="card-dark">
          <h3 className="text-lg font-medium mb-4 text-on-surface">Average Response Time</h3>
          <div style={{ height: '300px' }}>
            <Bar data={barData} options={barOptions} />
          </div>
        </div>
      </div>

      {/* Detailed Breakdown Table */}
      <div className="card-dark">
        <h3 className="text-lg font-medium mb-4 text-on-surface">Tool Usage Details</h3>
        <div className="overflow-x-auto">
          <table className="min-w-full divide-y divide-white/10">
            <thead className="bg-surface-container-low">
              <tr>
                <th className="px-6 py-3 text-left text-xs font-medium text-on-surface-variant uppercase tracking-wider">
                  Tool Name
                </th>
                <th className="px-6 py-3 text-left text-xs font-medium text-on-surface-variant uppercase tracking-wider">
                  Requests
                </th>
                <th className="px-6 py-3 text-left text-xs font-medium text-on-surface-variant uppercase tracking-wider">
                  Success Rate
                </th>
                <th className="px-6 py-3 text-left text-xs font-medium text-on-surface-variant uppercase tracking-wider">
                  Avg Response Time
                </th>
                <th className="px-6 py-3 text-left text-xs font-medium text-on-surface-variant uppercase tracking-wider">
                  Errors
                </th>
                <th className="px-6 py-3 text-left text-xs font-medium text-on-surface-variant uppercase tracking-wider">
                  Share
                </th>
              </tr>
            </thead>
            <tbody className="divide-y divide-white/10">
              {toolUsage.map((tool, index) => (
                <tr key={tool.tool_name} className="hover:bg-surface-container-low">
                  <td className="px-6 py-4 whitespace-nowrap">
                    <div className="flex items-center">
                      <div
                        className="w-3 h-3 rounded-full mr-3"
                        style={{ backgroundColor: colors[index % colors.length] }}
                      />
                      <div className="text-sm font-medium text-on-surface">
                        {formatToolName(tool.tool_name)}
                      </div>
                    </div>
                  </td>
                  <td className="px-6 py-4 whitespace-nowrap text-sm text-on-surface">
                    {tool.request_count.toLocaleString()}
                  </td>
                  <td className="px-6 py-4 whitespace-nowrap">
                    <div className="flex items-center">
                      <div className="text-sm text-on-surface">
                        {tool.success_rate.toFixed(1)}%
                      </div>
                      <div className="ml-2 w-16 bg-surface-container-high rounded-full h-2">
                        <div
                          className={`h-2 rounded-full ${
                            tool.success_rate >= 95 ? 'bg-pierre-activity' :
                            tool.success_rate >= 90 ? 'bg-pierre-nutrition' : 'bg-pierre-red-400'
                          }`}
                          style={{ width: `${tool.success_rate}%` }}
                        />
                      </div>
                    </div>
                  </td>
                  <td className="px-6 py-4 whitespace-nowrap text-sm text-on-surface">
                    {tool.average_response_time.toFixed(0)}ms
                  </td>
                  <td className="px-6 py-4 whitespace-nowrap">
                    {(() => {
                      const errorCount = Math.round(tool.request_count * (100 - tool.success_rate) / 100);
                      return (
                        <span className={`inline-flex px-2 py-1 text-xs font-medium rounded-full ${
                          errorCount === 0
                            ? 'bg-pierre-activity/20 text-pierre-activity border border-pierre-activity/30'
                            : errorCount < 10
                            ? 'bg-pierre-nutrition/20 text-pierre-nutrition border border-pierre-nutrition/30'
                            : 'bg-pierre-red-500/20 text-pierre-red-400 border border-pierre-red-500/30'
                        }`}>
                          {errorCount}
                        </span>
                      );
                    })()}
                  </td>
                  <td className="px-6 py-4 whitespace-nowrap text-sm text-on-surface">
                    {(() => {
                      const totalRequests = toolUsage?.reduce((sum, t) => sum + t.request_count, 0) || 1;
                      const percentage = (tool.request_count / totalRequests) * 100;
                      return percentage.toFixed(1);
                    })()}%
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      {/* Quick Stats */}
      <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
        <div className="stat-card-dark">
          <div className="text-2xl font-bold text-pierre-violet">
            {toolUsage.length}
          </div>
          <div className="text-sm text-on-surface-variant">Tools Used</div>
        </div>
        <div className="stat-card-dark">
          <div className="text-2xl font-bold text-pierre-cyan">
            {toolUsage.reduce((sum, tool) => sum + tool.request_count, 0).toLocaleString()}
          </div>
          <div className="text-sm text-on-surface-variant">Total Requests</div>
        </div>
        <div className="stat-card-dark">
          <div className="text-2xl font-bold text-pierre-activity">
            {(toolUsage.reduce((sum, tool) => sum + tool.success_rate * tool.request_count, 0) /
             toolUsage.reduce((sum, tool) => sum + tool.request_count, 0)).toFixed(1)}%
          </div>
          <div className="text-sm text-on-surface-variant">Overall Success Rate</div>
        </div>
        <div className="stat-card-dark">
          <div className="text-2xl font-bold text-on-surface">
            {(toolUsage.reduce((sum, tool) => sum + tool.average_response_time * tool.request_count, 0) /
             toolUsage.reduce((sum, tool) => sum + tool.request_count, 0)).toFixed(0)}ms
          </div>
          <div className="text-sm text-on-surface-variant">Avg Response Time</div>
        </div>
      </div>
    </div>
  );
}
