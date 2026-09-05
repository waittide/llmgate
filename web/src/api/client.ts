import { QueryClient } from '@tanstack/react-query';

// queryClient 管理整个前端应用共享的查询缓存和请求状态。
export const queryClient = new QueryClient({
    defaultOptions: {
        queries: {
            staleTime: 60 * 1000,
            refetchOnWindowFocus: false,
        },
    },
});

type RequestOptions = {
    method?: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';
    body?: unknown;
    dispatchUnauthorized?: boolean;
    signal?: AbortSignal;
};

export class ApiError extends Error {
    status: number;

    constructor(status: number, message: string) {
        super(message);
        this.name = 'ApiError';
        this.status = status;
    }
}

export const apiUnauthorizedEvent = 'api:unauthorized';

let authToken: string | null = null;
let apiKey: string | null = null;

export function setAuthToken(value: string | null) {
    authToken = value;
}

export function setAPIKey(value: string | null) {
    apiKey = value;
}

function getActiveBearer(): string | null {
    if (apiKey || authToken) return apiKey || authToken;
    try {
        if (typeof localStorage !== 'undefined') {
            const raw = localStorage.getItem('auth-storage');
            if (raw) {
                const parsed = JSON.parse(raw);
                const token = parsed?.state?.token;
                if (token && typeof token === 'string') {
                    if (parsed?.state?.isAPIKeyAuth) {
                        apiKey = token;
                    } else {
                        authToken = token;
                    }
                    return token;
                }
            }
        }
    } catch {
        // ignore storage parse error
    }
    return null;
}

export async function apiRequest<T>(path: string, options: RequestOptions = {}): Promise<T> {
    const headers = new Headers(options.body === undefined ? undefined : { 'Content-Type': 'application/json' });
    const bearer = getActiveBearer();
    if (bearer) headers.set('Authorization', `Bearer ${bearer}`);

    const response = await fetch(path, {
        method: options.method ?? 'GET',
        headers,
        body: options.body === undefined ? undefined : JSON.stringify(options.body),
        credentials: 'include',
        signal: options.signal,
    });
    const data = await response.json().catch(() => null) as { message?: string; data?: T } | null;
    if (!response.ok) {
        if (response.status === 401 && options.dispatchUnauthorized !== false && typeof window !== 'undefined') {
            window.dispatchEvent(new Event(apiUnauthorizedEvent));
        }
        throw new ApiError(response.status, data?.message || `Request failed: ${response.status}`);
    }
    return data?.data as T;
}

// openSseStream 以 fetch + ReadableStream 打开 SSE 连接（可携带 Authorization 请求头，
// 规避原生 EventSource 无法自定义请求头的限制）。连接断开后每 3 秒自动重连，
// 直至调用方执行返回的取消函数；每个完整事件（空行分隔）回调一次。
export function openSseStream(
    path: string,
    onEvent: (eventType: string, data: string) => void,
    onError?: (error: Error) => void,
): () => void {
    const controller = new AbortController();
    let reconnectTimer: number | undefined;
    let closed = false;

    const connect = async () => {
        if (closed) return;
        try {
            const headers = new Headers();
            const bearer = getActiveBearer();
            if (bearer) headers.set('Authorization', `Bearer ${bearer}`);
            const response = await fetch(path, {
                headers,
                credentials: 'include',
                signal: controller.signal,
            });
            if (!response.ok || !response.body) {
                throw new Error(`SSE 连接失败: ${response.status}`);
            }
            const reader = response.body.getReader();
            const decoder = new TextDecoder();
            let buffer = '';
            let done = false;
            while (!done) {
                const chunk = await reader.read();
                done = chunk.done;
                buffer += decoder.decode(chunk.value ?? new Uint8Array(), { stream: true });
                // 事件以空行分隔（兼容 \r\n\r\n）
                let boundary = buffer.search(/\r?\n\r?\n/);
                while (boundary >= 0) {
                    const block = buffer.slice(0, boundary);
                    const separator = /^\r?\n\r?\n/.exec(buffer.slice(boundary))?.[0] ?? '';
                    buffer = buffer.slice(boundary + separator.length);
                    let eventType = 'message';
                    const dataLines: string[] = [];
                    for (const line of block.split('\n')) {
                        if (line.startsWith(':')) continue; // 注释/保活行
                        if (line.startsWith('event:')) eventType = line.slice(6).trim();
                        else if (line.startsWith('data:')) dataLines.push(line.slice(5).trim());
                    }
                    if (dataLines.length > 0) onEvent(eventType, dataLines.join('\n'));
                    boundary = buffer.search(/\r?\n\r?\n/);
                }
            }
        } catch (error) {
            onError?.(error instanceof Error ? error : new Error(String(error)));
            if (closed) return;
        }
        // 断线/流结束后延迟重连
        reconnectTimer = window.setTimeout(() => {
            void connect();
        }, 3000);
    };

    void connect();
    return () => {
        closed = true;
        window.clearTimeout(reconnectTimer);
        controller.abort();
    };
}
