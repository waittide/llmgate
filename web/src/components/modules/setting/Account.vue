<script setup lang="ts">
import { ref } from 'vue';
import {
    LuUser,
    LuKeyRound,
    LuLock,
    LuEye,
    LuEyeOff,
    LuPencil,
    LuCheck,
    LuX,
    LuLoader,
} from 'vue-icons-plus/lu';
import { motion, AnimatePresence } from 'motion-v';
import { Input } from '@/components/ui/input';
import { useChangeUsername, useChangePassword } from '@/api/user';
import { logout } from '@/stores/auth';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';

const { t } = useTranslations('setting');

// 当前展开的表单, 为空时仅显示两行入口
const editing = ref<'username' | 'password' | null>(null);

// 修改用户名状态
const changeUsername = useChangeUsername();
const newUsername = ref('');

// 修改密码状态
const changePassword = useChangePassword();
const oldPassword = ref('');
const newPassword = ref('');
const confirmPassword = ref('');
const oldVisible = ref(false);
const newVisible = ref(false);
const confirmVisible = ref(false);

const openUsernameForm = () => {
    newUsername.value = '';
    editing.value = 'username';
};

const openPasswordForm = () => {
    oldPassword.value = '';
    newPassword.value = '';
    confirmPassword.value = '';
    oldVisible.value = false;
    newVisible.value = false;
    confirmVisible.value = false;
    editing.value = 'password';
};

const closeForm = () => {
    editing.value = null;
};

const handleUsernameSubmit = () => {
    const trimmed = newUsername.value.trim();
    if (!trimmed) {
        toast.error(t('account.username.empty'));
        return;
    }

    changeUsername.mutate(
        { newUsername: trimmed },
        {
            onSuccess: () => {
                toast.success(t('account.username.success'));
                closeForm();
                setTimeout(() => logout(), 1000);
            },
            onError: () => toast.error(t('account.username.failed')),
        }
    );
};

const handlePasswordSubmit = () => {
    if (!oldPassword.value) {
        toast.error(t('account.password.oldEmpty'));
        return;
    }
    if (!newPassword.value) {
        toast.error(t('account.password.newEmpty'));
        return;
    }
    if (newPassword.value !== confirmPassword.value) {
        toast.error(t('account.password.mismatch'));
        return;
    }
    if (newPassword.value.length < 6) {
        toast.error(t('account.password.tooShort'));
        return;
    }

    changePassword.mutate(
        { oldPassword: oldPassword.value, newPassword: newPassword.value },
        {
            onSuccess: () => {
                toast.success(t('account.password.success'));
                closeForm();
                setTimeout(() => logout(), 1000);
            },
            onError: () => toast.error(t('account.password.failed')),
        }
    );
};
</script>

<template>
    <div class="rounded-3xl border border-border bg-card p-6 space-y-5 relative">
        <h2 class="text-lg font-bold text-card-foreground flex items-center gap-2">
            <LuUser class="h-5 w-5" />
            {{ t('account.title') }}
        </h2>

        <!-- 修改用户名表单 -->
        <AnimatePresence>
            <motion.form
                v-if="editing === 'username'"
                key="username-form"
                layoutId="account-username"
                @submit.prevent="handleUsernameSubmit"
                :transition="{ type: 'spring', stiffness: 400, damping: 30 }"
                class="absolute left-1/2 top-1/2 z-20 grid w-[min(420px,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 gap-2 rounded-3xl border border-border bg-card p-5"
            >
                <Input
                    type="text"
                    v-model="newUsername"
                    :placeholder="t('account.username.placeholder')"
                    class="h-9 text-sm rounded-xl"
                    :disabled="changeUsername.isPending.value"
                />

                <div class="flex gap-2 pt-1">
                    <button
                        type="button"
                        @click="closeForm"
                        :disabled="changeUsername.isPending.value"
                        class="flex-1 h-9 flex items-center justify-center gap-1.5 rounded-xl bg-muted text-muted-foreground text-sm font-medium transition-all hover:bg-muted/80 active:scale-[0.98] disabled:opacity-50"
                    >
                        <LuX class="size-4" />
                        {{ t('account.cancel') }}
                    </button>
                    <button
                        type="submit"
                        :disabled="changeUsername.isPending.value || !newUsername.trim()"
                        class="flex-1 h-9 flex items-center justify-center gap-1.5 rounded-xl bg-primary text-primary-foreground text-sm font-medium transition-all hover:bg-primary/90 active:scale-[0.98] disabled:opacity-50"
                    >
                        <LuLoader v-if="changeUsername.isPending.value" class="size-4 animate-spin" />
                        <LuCheck v-else class="size-4" />
                        {{ changeUsername.isPending.value ? t('account.saving') : t('account.save') }}
                    </button>
                </div>
            </motion.form>
        </AnimatePresence>

        <!-- 修改密码表单 -->
        <AnimatePresence>
            <motion.form
                v-if="editing === 'password'"
                key="password-form"
                layoutId="account-password"
                @submit.prevent="handlePasswordSubmit"
                :transition="{ type: 'spring', stiffness: 400, damping: 30 }"
                class="absolute left-1/2 top-1/2 z-20 grid w-[min(420px,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 gap-2 rounded-3xl border border-border bg-card p-5"
            >
                <div class="relative">
                    <Input
                        :type="oldVisible ? 'text' : 'password'"
                        v-model="oldPassword"
                        :placeholder="t('account.password.oldPlaceholder')"
                        class="h-9 text-sm rounded-xl pr-10"
                        :disabled="changePassword.isPending.value"
                    />
                    <button
                        type="button"
                        @click="oldVisible = !oldVisible"
                        :aria-label="t('account.password.oldPlaceholder')"
                        class="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground transition-colors hover:text-foreground"
                    >
                        <LuEyeOff v-if="oldVisible" class="size-4" />
                        <LuEye v-else class="size-4" />
                    </button>
                </div>

                <div class="relative">
                    <Input
                        :type="newVisible ? 'text' : 'password'"
                        v-model="newPassword"
                        :placeholder="t('account.password.newPlaceholder')"
                        class="h-9 text-sm rounded-xl pr-10"
                        :disabled="changePassword.isPending.value"
                    />
                    <button
                        type="button"
                        @click="newVisible = !newVisible"
                        :aria-label="t('account.password.newPlaceholder')"
                        class="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground transition-colors hover:text-foreground"
                    >
                        <LuEyeOff v-if="newVisible" class="size-4" />
                        <LuEye v-else class="size-4" />
                    </button>
                </div>

                <div class="relative">
                    <Input
                        :type="confirmVisible ? 'text' : 'password'"
                        v-model="confirmPassword"
                        :placeholder="t('account.password.confirmPlaceholder')"
                        class="h-9 text-sm rounded-xl pr-10"
                        :disabled="changePassword.isPending.value"
                    />
                    <button
                        type="button"
                        @click="confirmVisible = !confirmVisible"
                        :aria-label="t('account.password.confirmPlaceholder')"
                        class="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground transition-colors hover:text-foreground"
                    >
                        <LuEyeOff v-if="confirmVisible" class="size-4" />
                        <LuEye v-else class="size-4" />
                    </button>
                </div>

                <div class="flex gap-2 pt-1">
                    <button
                        type="button"
                        @click="closeForm"
                        :disabled="changePassword.isPending.value"
                        class="flex-1 h-9 flex items-center justify-center gap-1.5 rounded-xl bg-muted text-muted-foreground text-sm font-medium transition-all hover:bg-muted/80 active:scale-[0.98] disabled:opacity-50"
                    >
                        <LuX class="size-4" />
                        {{ t('account.cancel') }}
                    </button>
                    <button
                        type="submit"
                        :disabled="changePassword.isPending.value || !oldPassword || !newPassword || !confirmPassword"
                        class="flex-1 h-9 flex items-center justify-center gap-1.5 rounded-xl bg-primary text-primary-foreground text-sm font-medium transition-all hover:bg-primary/90 active:scale-[0.98] disabled:opacity-50"
                    >
                        <LuLoader v-if="changePassword.isPending.value" class="size-4 animate-spin" />
                        <LuCheck v-else class="size-4" />
                        {{ changePassword.isPending.value ? t('account.saving') : t('account.password.change') }}
                    </button>
                </div>
            </motion.form>
        </AnimatePresence>

        <!-- 修改用户名 -->
        <div class="flex items-center justify-between gap-4">
            <div class="flex items-center gap-3">
                <LuKeyRound class="h-5 w-5 text-muted-foreground" />
                <span class="text-sm font-medium">{{ t('account.username.label') }}</span>
            </div>
            <motion.button
                type="button"
                layoutId="account-username"
                @click="openUsernameForm"
                :disabled="editing !== null"
                :aria-label="t('account.username.label')"
                class="flex size-8 items-center justify-center rounded-lg bg-muted/60 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground active:scale-95 disabled:opacity-50"
            >
                <LuPencil class="size-4" />
            </motion.button>
        </div>

        <!-- 修改密码 -->
        <div class="flex items-center justify-between gap-4">
            <div class="flex items-center gap-3">
                <LuLock class="h-5 w-5 text-muted-foreground" />
                <span class="text-sm font-medium">{{ t('account.password.label') }}</span>
            </div>
            <motion.button
                type="button"
                layoutId="account-password"
                @click="openPasswordForm"
                :disabled="editing !== null"
                :aria-label="t('account.password.label')"
                class="flex size-8 items-center justify-center rounded-lg bg-muted/60 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground active:scale-95 disabled:opacity-50"
            >
                <LuPencil class="size-4" />
            </motion.button>
        </div>
    </div>
</template>
