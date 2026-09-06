import { reactive, ref } from 'vue';
import { ApiError, apiRequest, apiUnauthorizedEvent, setAPIKey, setAuthToken } from '@/api/client';

/**
 * 用户登录响应
 */
interface UserLoginResponse {
    token: string;
    max_age: number;
}

/**
 * 认证状态（对应 React 版 useAuthStore + zustand persist）。
 * localStorage key/结构保持 auth-storage: {state:{token,isAPIKeyAuth}}，
 * 保证与 React 版会话互通。
 */
const STORAGE_KEY = 'llmgate-auth';

interface PersistedAuth {
    token: string | null;
    isAPIKeyAuth: boolean;
}

function readPersistedAuth(): PersistedAuth {
    try {
        const raw = localStorage.getItem(STORAGE_KEY);
        if (raw) {
            const parsed = JSON.parse(raw) as { state?: PersistedAuth };
            if (parsed?.state && typeof parsed.state.token === 'string') {
                return { token: parsed.state.token, isAPIKeyAuth: Boolean(parsed.state.isAPIKeyAuth) };
            }
        }
    } catch {
        // ignore storage parse error
    }
    return { token: null, isAPIKeyAuth: false };
}

function persistAuth() {
    try {
        localStorage.setItem(
            STORAGE_KEY,
            JSON.stringify({ state: { token: authStore.token, isAPIKeyAuth: authStore.isAPIKeyAuth } }),
        );
    } catch {
        // ignore storage write error
    }
}

export const authStore = reactive({
    isAuthenticated: false,
    isLoading: true,
    isAPIKeyAuth: false,
    token: readPersistedAuth().token,
});

// hydrated 表示持久化的认证状态是否已恢复；恢复前不校验 token。
export const hydrated = ref(false);

export function setAuth(jwtToken: string) {
    setAPIKey(null);
    setAuthToken(jwtToken);
    authStore.isAuthenticated = true;
    authStore.isAPIKeyAuth = false;
    authStore.token = jwtToken;
    authStore.isLoading = false;
    persistAuth();
}

export function setAPIKeyAuth(apiKey: string) {
    setAPIKey(apiKey);
    setAuthToken(null);
    authStore.isAuthenticated = true;
    authStore.isAPIKeyAuth = true;
    authStore.token = apiKey;
    authStore.isLoading = false;
    persistAuth();
}

export async function checkAuth(): Promise<void> {
    const { token, isAPIKeyAuth } = authStore;
    if (isAPIKeyAuth) {
        setAPIKey(token);
        setAuthToken(null);
    } else {
        setAPIKey(null);
        setAuthToken(token);
    }

    if (!token) {
        authStore.isAuthenticated = false;
        authStore.isLoading = false;
        return;
    }

    try {
        const endpoint = isAPIKeyAuth ? '/api/v1/apikey/login' : '/api/v1/user/status';
        await apiRequest<unknown>(endpoint, { dispatchUnauthorized: false });
        authStore.isAuthenticated = true;
        authStore.isLoading = false;
        authStore.token = token;
    } catch (err) {
        // 只有确定的 401 才使会话失效；刷新时的瞬时网络/服务端错误不能丢弃有效 token。
        if (err instanceof ApiError && err.status === 401) {
            logout();
        } else {
            authStore.isLoading = false;
            authStore.isAuthenticated = true;
        }
    }
}

export function logout() {
    setAPIKey(null);
    setAuthToken(null);
    authStore.isAuthenticated = false;
    authStore.isAPIKeyAuth = false;
    authStore.token = null;
    authStore.isLoading = false;
    persistAuth();
}

// 模块加载时恢复持久化状态并触发一次校验（对应 zustand persist 水合 + useAuth 副作用）。
const persisted = readPersistedAuth();
authStore.isAPIKeyAuth = persisted.isAPIKeyAuth;
if (persisted.token) {
    authStore.isAuthenticated = false; // 校验完成前由 isLoading 遮罩
    hydrated.value = true;
    void checkAuth();
} else {
    authStore.isLoading = false;
    hydrated.value = true;
}

// 全局未授权事件：任何请求返回 401 都视为会话失效。
if (typeof window !== 'undefined') {
    window.addEventListener(apiUnauthorizedEvent, () => logout());
}
