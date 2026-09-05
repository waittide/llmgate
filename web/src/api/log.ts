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

// 实时日志滑动窗口上限: 超过后在内存中丢弃最旧的条目, 防止长时间运行导致内存膨胀。
const MAX_RT_LOGS = 200;

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

// useLogs 订阅进程内日志概览，并按 RequestID 更新同一条记录。
export function useLogs() {
    const [logs, setLogs] = useState<RelayLogOverview[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [error, setError] = useState<Error | null>(null);

    useEffect(() => {
        const source = new EventSource('/api/v1/log/overview/stream', { withCredentials: true });

        source.onopen = () => {
            setError(null);
            setIsLoading(false);
        };
        source.addEventListener('overview', (event) => {
            let next: RelayLogOverview;
            try {
                next = JSON.parse((event as MessageEvent<string>).data) as RelayLogOverview;
            } catch {
                setError(new Error('Invalid log update'));
                return;
            }
            setIsLoading(false);
            setError(null);
            // 列表始终按 ID 倒序: 命中已有记录时原地替换, 新记录插入到首个更小 ID 之前,
            // 由此避免每条更新重排整个列表, 并保留未变更记录的引用以跳过卡片重渲染。
            setLogs((current) => {
                const index = current.findIndex((item) => item.id === next.id);
                if (index >= 0) {
                    const updated = current.slice();
                    updated[index] = next;
                    return updated;
                }
                const position = current.findIndex((item) => item.id < next.id);
                const nextList =
                    position < 0
                        ? [...current, next]
                        : [...current.slice(0, position), next, ...current.slice(position)];
                // 只保留最近 MAX_RT_LOGS 条, 丢弃最旧 (列表尾部, ID 最小)。
                if (nextList.length > MAX_RT_LOGS) {
                    return nextList.slice(0, MAX_RT_LOGS);
                }
                return nextList;
            });
        });
        source.onerror = () => {
            setIsLoading(false);
            setError(new Error('Log stream disconnected'));
        };

        return () => {
            source.close();
        };
    }, []);

    return { logs, isLoading, error };
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
