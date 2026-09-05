import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useState } from 'react';
import { apiRequest } from './client';

// RequestState 表示 Relay 请求的实时状态。
export type RequestState = 'running' | 'committed' | 'success' | 'failed' | 'canceled';

// RelayUsage 保存请求结束后确认的统一 Token 用量。
export interface RelayUsage {
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
    prompt_tokens_details: {
        cached_tokens: number;
        write_cached_tokens?: number;
    } | null;
}
// RoundLog 表示一轮上游渠道尝试的完整上下文与结果。
export interface RoundLog {
    round: number;
    channel_name: string;
    model_name: string;
    request_url: string;
    request_headers: string;
    request_body: string;
    response_url: string;
    response_headers: string;
    response_body: string;
    status: number;
    duration_ms: number;
    error?: string;
}


// RelayLogOverview 是请求状态流发送的完整进程内请求状态。
export interface RelayLogOverview {
    id: number;
    status: RequestState;
    started_at: string;
    duration: number;
    model: string;
    usage: RelayUsage;
    cost: number;
    round: number;
    target_channel: string;
    target_model: string;
    sending: boolean;
    error?: string;
    rounds?: RoundLog[];
}
// useClearLogs 清空已完成的内存日志。
export function useClearLogs() {
    return useMutation({
        mutationFn: () => apiRequest<null>('/api/v1/log/clear', { method: 'DELETE' }),
    });
}

// useStopRound 中止指定请求当前轮次匹配的上游调用。
export function useStopRound() {
    return useMutation({
        mutationFn: ({ requestId, round }: { requestId: number; round: number }) =>
            apiRequest<null>(`/api/v1/log/${requestId}/${round}/stop`, { method: 'POST' }),
    });
}
// useLogRequestBody 在调用方启用时按需获取指定日志的请求体。
export function useLogRequestBody(id: number, startedAt: string, enabled: boolean) {
    return useQuery({
        queryKey: ['logs', id, startedAt, 'request-body'],
        queryFn: () => apiRequest<string>(`/api/v1/log/${id}/request-body`),
        enabled,
        staleTime: Infinity,
    });
}

// useLogResponseBody 在调用方启用时获取指定日志的最终响应体。
export function useLogResponseBody(id: number, startedAt: string, enabled: boolean) {
    return useQuery({
        queryKey: ['logs', id, startedAt, 'response-body'],
        queryFn: () => apiRequest<string>(`/api/v1/log/${id}/response-body`),
        enabled,
        staleTime: Infinity,
    });
}

// PersistentLogDetail 是持久化请求日志的完整内容, 含请求/响应两侧的 URL、Header 与 Body。
export interface PersistentLogDetail extends RelayLogOverview {
    request_url: string;
    request_headers: string;
    request_body: string;
    response_url: string;
    response_headers: string;
    response_body: string;
    rounds_detail?: string;
}

// LogListResponse 是持久化日志分页列表的响应。
export interface LogListResponse {
    total: number;
    items: RelayLogOverview[];
}

// usePersistentLogs 按分页与过滤条件查询数据库中的持久化请求日志。
export function usePersistentLogs(params: { page: number; pageSize: number; model: string; status: string; keyword: string }) {
    return useQuery({
        queryKey: ['logs', 'list', params],
        queryFn: async () => {
            const q = new URLSearchParams();
            q.set('page', String(params.page));
            q.set('page_size', String(params.pageSize));
            if (params.model) q.set('model', params.model);
            if (params.status) q.set('status', params.status);
            if (params.keyword) q.set('keyword', params.keyword);
            return apiRequest<LogListResponse>(`/api/v1/log/list?${q.toString()}`);
        },
    });
}

// useLogDetail 获取指定持久化日志的完整详情(URL/Header/Body)。
export function useLogDetail(id: number, enabled: boolean) {
    return useQuery({
        queryKey: ['logs', id, 'detail'],
        queryFn: () => apiRequest<PersistentLogDetail>(`/api/v1/log/${id}`),
        enabled,
    });
}

// useLogListLiveSync 订阅概览 SSE: 任一请求日志被创建/更新(每次广播即一条日志的
// 创建或状态变更)时, 防抖失效 logs 相关查询, 使日志页无需手动刷新即可看到
// 实时条目与完成后的最终数据; 防抖把同轮的多次运行态广播合并为一次刷新。
export function useLogListLiveSync() {
    const queryClient = useQueryClient();
    useEffect(() => {
        const source = new EventSource('/api/v1/log/overview/stream', { withCredentials: true });
        let timer: number | undefined;
        source.addEventListener('overview', () => {
            window.clearTimeout(timer);
            timer = window.setTimeout(() => {
                void queryClient.invalidateQueries({ queryKey: ['logs'] });
            }, 500);
        });
        source.onerror = () => {
            // 断线时 EventSource 自动重连, 无需额外处理
        };
        return () => {
            window.clearTimeout(timer);
            source.close();
        };
    }, [queryClient]);
}
