import { defaultTheme } from '@vuepress/theme-default'
import { defineUserConfig } from 'vuepress'
import { viteBundler } from '@vuepress/bundler-vite'

export default defineUserConfig({
  locales: {
    '/': {
      lang: 'zh-CN',
      title: 'Ormer',
      description: '极简、高性能的 Rust ORM 框架'
    },
    '/en/': {
      lang: 'en-US',
      title: 'Ormer',
      description: 'Minimal, high-performance Rust ORM framework'
    }
  },

  theme: defaultTheme({
    locales: {
      '/': {
        selectLanguageName: '简体中文',
        selectLanguageText: 'Languages',
        selectLanguageAriaLabel: 'Select language',
        //logo: 'https://vuejs.press/images/hero.png',
        navbar: [
          '/',
          '/guide/01_quick_start',
          {
            text: 'GitHub',
            link: 'https://github.com/fawdlstty/ormer'
          }
        ],
        sidebar: {
          '/guide/': [
            "00_introduction", "01_quick_start", "02_model_definition",
            "03_database_connection", "04_crud_operations", "05_query_builder",
            "06_advanced_queries", "07_transactions", "08_connection_pool"
          ]
        }
      },
      '/en/': {
        selectLanguageName: 'English',
        selectLanguageText: 'Languages',
        selectLanguageAriaLabel: 'Select language',
        //logo: 'https://vuejs.press/images/hero.png',
        navbar: [
          '/en/',
          '/en/guide/01_quick_start.html',
          {
            text: 'GitHub',
            link: 'https://github.com/fawdlstty/ormer'
          }
        ],
        sidebar: {
          '/en/guide/': [
            "00_introduction", "01_quick_start", "02_model_definition",
            "03_database_connection", "04_crud_operations", "05_query_builder",
            "06_advanced_queries", "07_transactions", "08_connection_pool"
          ]
        }
      }
    }
  }),

  bundler: viteBundler(),
})
