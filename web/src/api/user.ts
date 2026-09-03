import { useEffect, useState } from 'react';
import { useMutation } from '@tanstack/react-query';
import { create } from 'zustand';
import { persist } from 'zustand/middleware';
import { ApiError, apiRequest, apiUnauthorizedEvent, setAPIKey, setAuthToken } from './client';

/**
 * 用户登录请求
 */
interface UserLoginRequest {
    username: string;
    password: string;
    expire: number; // 登录状态过期时间，正数为秒，-1 表示 30 天。
}

/**
 * 认证状态 Store
 */
interface AuthState {
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

/**
 * 认证状态管理 Store（使用 zustand + persist）
 */
export const useAuthStore = create<AuthState>()(
    persist(
        (set, get) => ({
            isAuthenticated: false,
            isLoading: true,
            isAPIKeyAuth: false,
            token: null,

            setAuth: (jwtToken: string) => {
                setAPIKey(null);
                setAuthToken(jwtToken);
                set({
                    isAuthenticated: true,
                    isAPIKeyAuth: false,
                    token: jwtToken,
                    isLoading: false
                });
            },

            setAPIKeyAuth: (apiKey: string) => {
                setAPIKey(apiKey);
                setAuthToken(null);
                set({
                    isAuthenticated: true,
                    isAPIKeyAuth: true,
                    token: apiKey,
                    isLoading: false
                });
            },

            checkAuth: async () => {
                const { token, isAPIKeyAuth } = get();
                if (isAPIKeyAuth) {
                    setAPIKey(token);
                    setAuthToken(null);
                } else {
                    setAPIKey(null);
                    setAuthToken(token);
                }

                if (!token) {
                    set({ isAuthenticated: false, isLoading: false });
                    return;
                }

                try {
                    const endpoint = isAPIKeyAuth ? '/api/v1/apikey/login' : '/api/v1/user/status';
                    await apiRequest<unknown>(endpoint, { dispatchUnauthorized: false });
                    set({
                        isAuthenticated: true,
                        isLoading: false,
                        token
                    });
                } catch (err) {
                    // Only a definitive 401 invalidates the session. A transient
                    // network/server error on refresh must not drop a valid token.
                    if (err instanceof ApiError && err.status === 401) {
                        get().logout();
                    } else {
                        set({ isLoading: false, isAuthenticated: true });
                    }
                }
            },

            logout: () => {
                setAPIKey(null);
                setAuthToken(null);
                set({
                    isAuthenticated: false,
                    isAPIKeyAuth: false,
                    token: null,
                    isLoading: false
                });
            }
        }),
        {
            name: 'auth-storage',
            partialize: (state) => ({
                token: state.token,
                isAPIKeyAuth: state.isAPIKeyAuth,
            })
        }
    )
);

/**
 * 用户登录 Hook
 * 
 * @example
 * const login = useLogin();
 * login.mutate({ username: 'admin', password: '123456', expire: 86400 });
 * 
 * if (login.isPending) return <Loading />;
 * if (login.isError) return <Error message={login.error.message} />;
 */
export function useLogin() {
    const { setAuth } = useAuthStore();

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
 * if (auth.isAuthenticated) {
 *   // 已登录
 * }
 * 
 * auth.logout(); // 登出
 */
export function useAuth() {
    const store = useAuthStore();
    const { checkAuth, isLoading } = store;
    // hydrated 表示持久化的认证状态是否已经恢复, 恢复前不校验 token。
    const [hydrated, setHydrated] = useState(false);

    // 全局未授权事件: 任何请求返回 401 都视为会话失效。
    useEffect(() => {
        const handleUnauthorized = () => useAuthStore.getState().logout();
        window.addEventListener(apiUnauthorizedEvent, handleUnauthorized);
        return () => window.removeEventListener(apiUnauthorizedEvent, handleUnauthorized);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // 等待 persist 水合完成后再校验 token, 避免刷新时读到尚未恢复的空 token
    // 而误判为未登录/已退出。水合在微任务中完成, 这里先查 hasHydrated 兜底。
    useEffect(() => {
        if (useAuthStore.persist.hasHydrated()) {
            setHydrated(true);
            return;
        }
        const unsubscribe = useAuthStore.persist.onFinishHydration(() => setHydrated(true));
        return unsubscribe;
    }, []);

    useEffect(() => {
        if (hydrated) {
            void checkAuth();
        }
    }, [hydrated, checkAuth]);

    return {
        isAuthenticated: store.isAuthenticated,
        isAPIKeyAuth: store.isAPIKeyAuth,
        isLoading: store.isLoading,
        logout: store.logout,
    };
}
