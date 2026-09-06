<script setup lang="ts">
import { computed, ref } from "vue"
import { useTranslations } from "@/composables/i18n"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Field, FieldDescription, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { useLogin } from "@/api/user"
import { useAPIKeyLogin } from "@/api/apikey"
import Logo from "@/components/modules/logo/Logo.vue"
import { LuKeyRound, LuUser } from "vue-icons-plus/lu"
import {
  Tabs,
  TabsList,
  TabsTrigger,
  TabsContent,
} from "@/components/ui/tabs"

type LoginMode = 'user' | 'apikey'

// LoginForm 渲染用户密码和 API Key 两种登录表单。
const { t } = useTranslations('login')
const mode = ref<LoginMode>('user')
const username = ref("")
const password = ref("")
const trustDevice = ref(false) // trustDevice 表示是否请求后端签发 30 天登录凭证。
const apiKey = ref("")
const error = ref<string | null>(null)

const loginMutation = useLogin()
const apiKeyLoginMutation = useAPIKeyLogin()

const handleSubmit = async (e: Event) => {
  e.preventDefault()
  error.value = null

  try {
    if (mode.value === 'user') {
      await loginMutation.mutateAsync({
        username: username.value,
        password: password.value,
        expire: trustDevice.value ? -1 : 86400,
      })
    } else {
      await apiKeyLoginMutation.mutateAsync(apiKey.value)
    }
  } catch (err: unknown) {
    const errorMessage = err instanceof Error ? err.message : t('error.generic')
    error.value = errorMessage
  }
}

const isPending = computed(() => loginMutation.isPending.value || apiKeyLoginMutation.isPending.value)

const handleModeChange = (value: string | number) => {
  mode.value = value as LoginMode
  error.value = null
}
</script>
<template>
  <div class="min-h-screen flex animate-in items-center justify-center px-6 text-foreground fade-in duration-300">
    <div class="w-full max-w-sm space-y-8">
      <header class="flex flex-col items-center gap-3">
        <Logo :size="48" />
        <h1 class="text-2xl font-bold">Octopus</h1>
      </header>

      <Tabs v-model="mode" @value-change="handleModeChange">
        <TabsList class="flex w-full rounded-2xl bg-muted p-1">
          <TabsTrigger
            value="user"
            class="w-full flex items-center justify-center gap-2 py-2.5 px-4 rounded-xl text-sm font-medium transition-colors data-[state=active]:text-foreground data-[state=inactive]:text-muted-foreground data-[state=inactive]:hover:text-foreground"
          >
            <LuUser class="w-4 h-4" />
            {{ t('mode.user') }}
          </TabsTrigger>
          <TabsTrigger
            value="apikey"
            class="w-full flex items-center justify-center gap-2 py-2.5 px-4 rounded-xl text-sm font-medium transition-colors data-[state=active]:text-foreground data-[state=inactive]:text-muted-foreground data-[state=inactive]:hover:text-foreground"
          >
            <LuKeyRound class="w-4 h-4" />
            {{ t('mode.apikey') }}
          </TabsTrigger>
        </TabsList>

        <form @submit="handleSubmit" class="space-y-6 pt-2">
          <div class="-mx-3 p-3 py-6">
            <TabsContent value="user" class="space-y-6" style="overflow: visible">
              <Field>
                <FieldLabel for="username">{{ t('username') }}</FieldLabel>
                <Input
                  id="username"
                  type="text"
                  :placeholder="t('usernamePlaceholder')"
                  v-model="username"
                  :required="mode === 'user'"
                  :disabled="isPending"
                />
              </Field>
              <Field>
                <FieldLabel for="password">{{ t('password') }}</FieldLabel>
                <Input
                  id="password"
                  type="password"
                  :placeholder="t('passwordPlaceholder')"
                  v-model="password"
                  :required="mode === 'user'"
                  :disabled="isPending"
                />
              </Field>
              <Field orientation="horizontal" :data-disabled="isPending">
                <Checkbox
                  id="trust-device"
                  v-model="trustDevice"
                  :disabled="isPending"
                />
                <FieldLabel for="trust-device" class="text-muted-foreground">
                  {{ t('trustDevice') }}
                </FieldLabel>
              </Field>
            </TabsContent>
            <TabsContent value="apikey" style="overflow: visible">
              <Field>
                <FieldLabel for="apikey">{{ t('apikey') }}</FieldLabel>
                <Input
                  id="apikey"
                  type="password"
                  :placeholder="t('apikeyPlaceholder')"
                  v-model="apiKey"
                  :required="mode === 'apikey'"
                  :disabled="isPending"
                />
              </Field>
            </TabsContent>
          </div>

          <FieldDescription v-if="error" class="text-destructive">{{ error }}</FieldDescription>

          <Button type="submit" :disabled="isPending" class="w-full">
            {{ isPending ? t('button.loading') : t('button.submit') }}
          </Button>
        </form>
      </Tabs>
    </div>
  </div>
</template>
