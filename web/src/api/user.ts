import { computed, onMounted, onUnmounted, watch } from 'vue';
import { useMutation } from '@tanstack/vue-query';
import { apiRequest, apiUnauthorizedEvent, setAPIKey, setAuthToken } from './client';
import { authStore, checkAuth, hydrated, logout, setAPIKeyAuth, setAuth } from '@/stores/auth';

/**
 * 用户登录请求
 */
export interface UserLoginRequest {
    username: string;
    password: string;
    expire: number; // 登录状态过期时间，正数为秒，-1 表示 30 天。
}

/**
 * 认证状态 Store 结构
 */
export interface AuthState {
    isAuthenticated: boolean;
    isLoading: boolean;
    isAPIKeyAuth: boolean;
    token: string | null;

    // Actions
    setAuth: (jwtToken: string) => void;
    setAPIKeyAuth: (apiKey: string) => void;
    checkAuth: () => Promise<void>;
    logout: () => void;
}

export { authStore, checkAuth, hydrated, logout, setAPIKeyAuth, setAuth };

/**
 * 兼容 React 版 useAuthStore 导出的 helper
 */
export function useAuthStore() {
    return {
        ...authStore,
        setAuth,
        setAPIKeyAuth,
        checkAuth,
        logout,
    };
}

/**
 * 用户登录 Hook
 * 
 * @example
 * const login = useLogin();
 * login.mutate({ username: 'admin', password: '123456', expire: 86400 });
 */
export function useLogin() {
    return useMutation({
        mutationFn: async (data: UserLoginRequest) => {
            setAPIKey(null);
            setAuthToken(null);
            return apiRequest<{ token: string; max_age: number }>('/api/v1/user/login', {
                method: 'POST',
                body: data,
                dispatchUnauthorized: false,
            });
        },
        onSuccess: (res) => {
            if (res?.token) {
                setAuth(res.token);
            }
        },
    });
}

/**
 * 修改密码 Hook
 * 
 * @example
 * const changePassword = useChangePassword();
 * changePassword.mutate({ oldPassword: '123', newPassword: '456' });
 */
export function useChangePassword() {
    return useMutation({
        mutationFn: (data: { oldPassword: string; newPassword: string }) =>
            apiRequest<string>('/api/v1/user/change-password', {
                method: 'POST',
                body: {
                    old_password: data.oldPassword,
                    new_password: data.newPassword,
                },
            }),
    });
}

/**
 * 修改用户名 Hook
 * 
 * @example
 * const changeUsername = useChangeUsername();
 * changeUsername.mutate({ newUsername: 'newname' });
 */
export function useChangeUsername() {
    return useMutation({
        mutationFn: (data: { newUsername: string }) =>
            apiRequest<string>('/api/v1/user/change-username', {
                method: 'POST',
                body: { new_username: data.newUsername },
            }),
    });
}

/**
 * 认证状态和方法 Hook
 * 
 * @example
 * const auth = useAuth();
 * 
 * if (auth.isAuthenticated.value) {
 *   // 已登录
 * }
 * 
 * auth.logout(); // 登出
 */
export function useAuth() {
    onMounted(() => {
        const handleUnauthorized = () => logout();
        window.addEventListener(apiUnauthorizedEvent, handleUnauthorized);
        onUnmounted(() => window.removeEventListener(apiUnauthorizedEvent, handleUnauthorized));
    });

    watch(
        hydrated,
        (isHydrated) => {
            if (isHydrated) {
                void checkAuth();
            }
        },
        { immediate: true },
    );

    return {
        isAuthenticated: computed(() => authStore.isAuthenticated),
        isAPIKeyAuth: computed(() => authStore.isAPIKeyAuth),
        isLoading: computed(() => authStore.isLoading),
        logout,
    };
}
