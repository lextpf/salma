<div align="center">

# salma
**Wizardless FOMOD installer with automatic selection inference**

🎀 [Features](#features) | 💃 [Quick Start](#quick-start) | 📘 [Documentation](#documentation) | 🤝 [Contributing](./CONTRIBUTING.md)

![ReactJS](https://img.shields.io/badge/React-TSX-61DAFB.svg?style=flat&logo=data:image/svg+xml;base64,PHN2ZyB2aWV3Qm94PSIwIDAgMjQgMjQiIGZpbGw9Im5vbmUiIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyI+PGcgaWQ9IlNWR1JlcG9fYmdDYXJyaWVyIiBzdHJva2Utd2lkdGg9IjAiPjwvZz48ZyBpZD0iU1ZHUmVwb190cmFjZXJDYXJyaWVyIiBzdHJva2UtbGluZWNhcD0icm91bmQiIHN0cm9rZS1saW5lam9pbj0icm91bmQiPjwvZz48ZyBpZD0iU1ZHUmVwb19pY29uQ2FycmllciI+IDxwYXRoIGQ9Ik0xMi4wMDAyIDEyVjE0QzEzLjEwNDggMTQgMTQuMDAwMiAxMy4xMDQ1IDE0LjAwMDIgMTJIMTIuMDAwMlpNMTIuMDAwMiAxMkgxMC4wMDAyQzEwLjAwMDIgMTMuMTA0NSAxMC44OTU2IDE0IDEyLjAwMDIgMTRWMTJaTTEyLjAwMDIgMTJWOS45OTk5NUMxMC44OTU2IDkuOTk5OTUgMTAuMDAwMiAxMC44OTU0IDEwLjAwMDIgMTJIMTIuMDAwMlpNMTIuMDAwMiAxMkgxNC4wMDAyQzE0LjAwMDIgMTAuODk1NCAxMy4xMDQ4IDkuOTk5OTUgMTIuMDAwMiA5Ljk5OTk1VjEyWk0xMi4wMDAyIDEzSDEyLjAxMDJWMTFIMTIuMDAwMlYxM1pNMTQuODI4NiAxNC44Mjg0QzEyLjc1NzkgMTYuODk5MSAxMC41MzQ1IDE4LjM1NjYgOC42NDkwNyAxOS4wNjM2QzYuNjcwNzYgMTkuODA1NSA1LjQ1NzY0IDE5LjU5OTUgNC45MjkxMyAxOS4wNzFMMy41MTQ5MiAyMC40ODUyQzQuOTM5MDIgMjEuOTA5MyA3LjIzNDg4IDIxLjcyOTkgOS4zNTEzMiAyMC45MzYzQzExLjU2MDYgMjAuMTA3OCAxNC4wMTc4IDE4LjQ2NzcgMTYuMjQyOCAxNi4yNDI2TDE0LjgyODYgMTQuODI4NFpNNC45MjkxMyAxOS4wNzFDNC40MDA2MSAxOC41NDI1IDQuMTk0NjYgMTcuMzI5NCA0LjkzNjUzIDE1LjM1MTFDNS42NDM1OCAxMy40NjU2IDcuMTAxMDYgMTEuMjQyMiA5LjE3MTc3IDkuMTcxNTJMNy43NTc1NiA3Ljc1NzMxQzUuNTMyNSA5Ljk4MjM3IDMuODkyMzUgMTIuNDM5NSAzLjA2Mzg3IDE0LjY0ODhDMi4yNzAyIDE2Ljc2NTMgMi4wOTA4MSAxOS4wNjExIDMuNTE0OTIgMjAuNDg1Mkw0LjkyOTEzIDE5LjA3MVpNOS4xNzE3NyA5LjE3MTUyQzExLjI0MjUgNy4xMDA4MiAxMy40NjU4IDUuNjQzMzMgMTUuMzUxMyA0LjkzNjI4QzE3LjMyOTYgNC4xOTQ0MSAxOC41NDI3IDQuNDAwMzcgMTkuMDcxMyA0LjkyODg4TDIwLjQ4NTUgMy41MTQ2N0MxOS4wNjE0IDIuMDkwNTYgMTYuNzY1NSAyLjI2OTk2IDE0LjY0OTEgMy4wNjM2MkMxMi40Mzk4IDMuODkyMSA5Ljk4MjYyIDUuNTMyMjUgNy43NTc1NiA3Ljc1NzMxTDkuMTcxNzcgOS4xNzE1MlpNMTkuMDcxMyA0LjkyODg4QzE5LjU5OTggNS40NTc0IDE5LjgwNTcgNi42NzA1MSAxOS4wNjM5IDguNjQ4ODNDMTguMzU2OCAxMC41MzQzIDE2Ljg5OTMgMTIuNzU3NyAxNC44Mjg2IDE0LjgyODRMMTYuMjQyOCAxNi4yNDI2QzE4LjQ2NzkgMTQuMDE3NSAyMC4xMDggMTEuNTYwNCAyMC45MzY1IDkuMzUxMDhDMjEuNzMwMiA3LjIzNDY0IDIxLjkwOTYgNC45Mzg3OCAyMC40ODU1IDMuNTE0NjdMMTkuMDcxMyA0LjkyODg4Wk0xNC44Mjg2IDkuMTcxNTJDMTYuODk5MyAxMS4yNDIyIDE4LjM1NjggMTMuNDY1NiAxOS4wNjM5IDE1LjM1MTFDMTkuODA1NyAxNy4zMjk0IDE5LjU5OTggMTguNTQyNSAxOS4wNzEzIDE5LjA3MUwyMC40ODU1IDIwLjQ4NTJDMjEuOTA5NiAxOS4wNjExIDIxLjczMDIgMTYuNzY1MyAyMC45MzY1IDE0LjY0ODhDMjAuMTA4IDEyLjQzOTUgMTguNDY3OSA5Ljk4MjM3IDE2LjI0MjggNy43NTczMUwxNC44Mjg2IDkuMTcxNTJaTTE5LjA3MTMgMTkuMDcxQzE4LjU0MjcgMTkuNTk5NSAxNy4zMjk2IDE5LjgwNTUgMTUuMzUxMyAxOS4wNjM2QzEzLjQ2NTggMTguMzU2NiAxMS4yNDI1IDE2Ljg5OTEgOS4xNzE3NyAxNC44Mjg0TDcuNzU3NTYgMTYuMjQyNkM5Ljk4MjYyIDE4LjQ2NzcgMTIuNDM5OCAyMC4xMDc4IDE0LjY0OTEgMjAuOTM2M0MxNi43NjU1IDIxLjcyOTkgMTkuMDYxNCAyMS45MDkzIDIwLjQ4NTUgMjAuNDg1MkwxOS4wNzEzIDE5LjA3MVpNOS4xNzE3NyAxNC44Mjg0QzcuMTAxMDYgMTIuNzU3NyA1LjY0MzU4IDEwLjUzNDMgNC45MzY1MyA4LjY0ODgzQzQuMTk0NjYgNi42NzA1MSA0LjQwMDYxIDUuNDU3NCA0LjkyOTEzIDQuOTI4ODhMMy41MTQ5MSAzLjUxNDY3QzIuMDkwODEgNC45Mzg3OCAyLjI3MDIgNy4yMzQ2NCAzLjA2Mzg3IDkuMzUxMDhDMy44OTIzNSAxMS41NjA0IDUuNTMyNSAxNC4wMTc1IDcuNzU3NTYgMTYuMjQyNkw5LjE3MTc3IDE0LjgyODRaTTQuOTI5MTMgNC45Mjg4OEM1LjQ1NzY0IDQuNDAwMzcgNi42NzA3NiA0LjE5NDQxIDguNjQ5MDcgNC45MzYyOEMxMC41MzQ1IDUuNjQzMzMgMTIuNzU3OSA3LjEwMDgyIDE0LjgyODYgOS4xNzE1MkwxNi4yNDI4IDcuNzU3MzFDMTQuMDE3OCA1LjUzMjI1IDExLjU2MDYgMy44OTIxIDkuMzUxMzIgMy4wNjM2MkM3LjIzNDg4IDIuMjY5OTYgNC45MzkwMiAyLjA5MDU2IDMuNTE0OTEgMy41MTQ2N0w0LjkyOTEzIDQuOTI4ODhaIiBmaWxsPSIjZmZmZmZmIj48L3BhdGg+IDwvZz48L3N2Zz4=)
![Crow](https://img.shields.io/badge/Crow-HTTP-D97706.svg?style=flat&logo=data:image/svg+xml;base64,PHN2ZyBmaWxsPSIjZmZmZmZmIiB2aWV3Qm94PSIwIC02NCA2NDAgNjQwIiB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciPjxnIGlkPSJTVkdSZXBvX2JnQ2FycmllciIgc3Ryb2tlLXdpZHRoPSIwIj48L2c+PGcgaWQ9IlNWR1JlcG9fdHJhY2VyQ2FycmllciIgc3Ryb2tlLWxpbmVjYXA9InJvdW5kIiBzdHJva2UtbGluZWpvaW49InJvdW5kIj48L2c+PGcgaWQ9IlNWR1JlcG9faWNvbkNhcnJpZXIiPjxwYXRoIGQ9Ik01NDQgMzJoLTE2LjM2QzUxMy4wNCAxMi42OCA0OTAuMDkgMCA0NjQgMGMtNDQuMTggMC04MCAzNS44Mi04MCA4MHYyMC45OEwxMi4wOSAzOTMuNTdBMzAuMjE2IDMwLjIxNiAwIDAgMCAwIDQxNy43NGMwIDIyLjQ2IDIzLjY0IDM3LjA3IDQzLjczIDI3LjAzTDE2NS4yNyAzODRoOTYuNDlsNDQuNDEgMTIwLjFjMi4yNyA2LjIzIDkuMTUgOS40NCAxNS4zOCA3LjE3bDIyLjU1LTguMjFjNi4yMy0yLjI3IDkuNDQtOS4xNSA3LjE3LTE1LjM4TDMxMi45NCAzODRIMzUyYzEuOTEgMCAzLjc2LS4yMyA1LjY2LS4yOWw0NC41MSAxMjAuMzhjMi4yNyA2LjIzIDkuMTUgOS40NCAxNS4zOCA3LjE3bDIyLjU1LTguMjFjNi4yMy0yLjI3IDkuNDQtOS4xNSA3LjE3LTE1LjM4bC00MS4yNC0xMTEuNTNDNDg1Ljc0IDM1Mi44IDU0NCAyNzkuMjYgNTQ0IDE5MnYtODBsOTYtMTZjMC0zNS4zNS00Mi45OC02NC05Ni02NHptLTgwIDcyYy0xMy4yNSAwLTI0LTEwLjc1LTI0LTI0IDAtMTMuMjYgMTAuNzUtMjQgMjQtMjRzMjQgMTAuNzQgMjQgMjRjMCAxMy4yNS0xMC43NSAyNC0yNCAyNHoiPjwvcGF0aD48L2c+PC9zdmc+)
![Mo2](https://img.shields.io/badge/Mo2-Dll-64748B.svg?style=flat&logo=data:image/svg+xml;base64,PHN2ZyBmaWxsPSIjZmZmZmZmIiB2ZXJzaW9uPSIxLjEiIGlkPSJDYXBhXzEiIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgeG1sbnM6eGxpbms9Imh0dHA6Ly93d3cudzMub3JnLzE5OTkveGxpbmsiIHZpZXdCb3g9Ii0xMjIuNCAtMTIyLjQgODU2LjgwIDg1Ni44MCIgeG1sOnNwYWNlPSJwcmVzZXJ2ZSI+PGcgaWQ9IlNWR1JlcG9fYmdDYXJyaWVyIiBzdHJva2Utd2lkdGg9IjAiPjwvZz48ZyBpZD0iU1ZHUmVwb190cmFjZXJDYXJyaWVyIiBzdHJva2UtbGluZWNhcD0icm91bmQiIHN0cm9rZS1saW5lam9pbj0icm91bmQiPjwvZz48ZyBpZD0iU1ZHUmVwb19pY29uQ2FycmllciI+IDxnPiA8cGF0aCBkPSJNNDgyLjE4OCw4My4zMzNMMTg0LjYyMiwyMjMuMjI1djg5LjgzMmwtNTEuOTEtMjMuMDgydi04OS44MzJMNDMwLjI3OCw2MC4yNTJsLTk5Ljk0Ni00NC40MzkgYy0xMy4zODMtNS45NS0zNS4yODEtNS45NS00OC42NjQsMEwzNS41NTcsMTI1LjI0M0MxNS45NSwxMzMuOTYxLTAuMDUsMTU4LjY0OSwwLDE4MC4xMDdsMC42MDYsMjU2LjUzNCBjMC4wNTEsMjEuNjg2LDE2LjQwOCw0Ni40MDEsMzYuMzQ4LDU0LjkyNkwyODIuNDIsNTk2LjQ5OWMxMi45NDUsNS41MzQsMzQuMTI5LDUuNTM0LDQ3LjA3NSwwLjAwM2wyNDUuNTUtMTA0LjkzNiBjMTkuOTM5LTguNTIxLDM2LjI5Ny0zMy4yMzQsMzYuMzQ4LTU0LjkxOUw2MTIsMTgwLjEwN2MwLjA1MS0yMS40NTgtMTUuOTQ5LTQ2LjE0Ni0zNS41NTctNTQuODY0TDQ4Mi4xODgsODMuMzMzeiBNNTU2LjM5OCwyODguNjc1bC0xNC40MDMsNi42ODNsLTAuMjkyLDEwMS4zNTNjLTAuMDEzLDQuNDI5LTMuOTI1LDkuNzAxLTguNzI3LDExLjc3M2wtMjEuNTYzLDkuMzA5IGMtNC43MjcsMi4wNDEtOC41NTEsMC4xNDktOC41NTQtNC4yMjNsLTAuMDczLTEwMC4wMjFsLTEzLjk1MSw2LjQ3MmMtNi41NjIsMy4wNDQtMTAuNjY5LTEuNzI5LTcuNDExLTguNjAxbDMzLjM0OC03MC4zNTYgYzMuMzY2LTcuMTAyLDExLjgwNi0xMS4xOTksMTUuMTg0LTcuMzQ3bDM0LjIyMSwzOS4wMTJDNTY3LjU5MywyNzYuNjIzLDU2My4yNTcsMjg1LjQ5NCw1NTYuMzk4LDI4OC42NzV6IE00MTUuNTk2LDQ1MS40NDMgYzAuMDM3LDQuMjQzLTMuNTUsOS4yNC04LjAwMSwxMS4xNjJsLTE5Ljk5Niw4LjYzMmMtNC4zODUsMS44OTMtNy45NzIsMC4wMjktOC4wMjItNC4xNmwtMS4xNzEtOTUuODI2bC0xMi45MzgsNi4wMDIgYy02LjA4NSwyLjgyMy05Ljk2OC0xLjgwOC03LjAwNi04LjM0NGwzMC4zMS02Ni44ODFjMy4wNTctNi43NDcsMTAuODczLTEwLjU0MSwxNC4wNjItNi44MDVsMzIuMzAxLDM3LjgzNiBjMy4yMjYsMy43NzctMC43MTIsMTIuMjAyLTcuMDYyLDE1LjE0N2wtMTMuMzM4LDYuMTg4TDQxNS41OTYsNDUxLjQ0M3ogTTU4MC4yMDEsNDIzLjYxOWMtMC4wMTUsMi4yMjYtMi4wMTYsNC44NjUtNC40NjgsNS44OTYgbC0yMjguMzk1LDk1Ljk1Yy0yLjEzMSwwLjg5Ni0zLjg4NC0wLjA0My0zLjkxNS0yLjA5NmwtMC4xNzUtMTEuMTYyYy0wLjAzMi0yLjA1OCwxLjY3LTQuNDYzLDMuODA1LTUuMzcybDIyOC44MDItOTcuNDY3IGMyLjQ1NS0xLjA0Niw0LjQzOC0wLjA4Niw0LjQyMywyLjE0Nkw1ODAuMjAxLDQyMy42MTl6Ij48L3BhdGg+IDwvZz4gPC9nPjwvc3ZnPg==)
![Tailwind](https://img.shields.io/badge/Tailwind-CSS-2DD4BF.svg?style=flat&logo=data:image/svg+xml;base64,PHN2ZyB2aWV3Qm94PSItMS41IC0xLjUgMTguMDAgMTguMDAiIGZpbGw9Im5vbmUiIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyI+PGcgaWQ9IlNWR1JlcG9fYmdDYXJyaWVyIiBzdHJva2Utd2lkdGg9IjAiPjwvZz48ZyBpZD0iU1ZHUmVwb190cmFjZXJDYXJyaWVyIiBzdHJva2UtbGluZWNhcD0icm91bmQiIHN0cm9rZS1saW5lam9pbj0icm91bmQiPjwvZz48ZyBpZD0iU1ZHUmVwb19pY29uQ2FycmllciI+IDxwYXRoIGQ9Ik03LjUwMDA2IDIuNUM2LjQ3NDA5IDIuNSA1LjU5MjAzIDIuNzc2OTEgNC44OTk2NiAzLjM3MDM3QzQuMjEyMjcgMy45NTk1NiAzLjc2MjU5IDQuODE3MjkgMy41MTMxNCA1Ljg4NjM4QzMuNDU4NjkgNi4xMTk3IDMuNTc3NDIgNi4zNTg4NSAzLjc5NjE5IDYuNDU2NTRDNC4wMTQ5NiA2LjU1NDIzIDQuMjcyMjggNi40ODMgNC40MDk2NyA2LjI4NjcyQzQuNzI2MyA1LjgzNDQgNS4wNDI0NCA1LjU2MjYxIDUuMzQ2MiA1LjQyMzEzQzUuNjQwMzggNS4yODgwNSA1Ljk1NzQ4IDUuMjYwNjggNi4zMjA2OSA1LjM1Nzk3QzYuNjg3MjMgNS40NTYxNSA2Ljk3MDk3IDUuNzQzNjkgNy40MTY0MyA2LjIyODE2TDcuNDMwODIgNi4yNDM4MkM3Ljc2NjYxIDYuNjA5MDUgOC4xNzYyMyA3LjA1NDYgOC43MzY0OSA3LjQwMDI4QzkuMzE3ODUgNy43NTg5OCAxMC4wNDEzIDcuOTk5OTkgMTEuMDAwMSA3Ljk5OTk5QzEyLjAyNiA3Ljk5OTk5IDEyLjkwODEgNy43MjMwNyAxMy42MDA1IDcuMTI5NjJDMTQuMjg3OCA2LjU0MDQzIDE0LjczNzUgNS42ODI3IDE0Ljk4NyA0LjYxMzYxQzE1LjA0MTQgNC4zODAyOSAxNC45MjI3IDQuMTQxMTQgMTQuNzAzOSA0LjA0MzQ1QzE0LjQ4NTIgMy45NDU3NiAxNC4yMjc4IDQuMDE2OTggMTQuMDkwNCA0LjIxMzI2QzEzLjc3MzggNC42NjU1OSAxMy40NTc3IDQuOTM3MzcgMTMuMTUzOSA1LjA3Njg2QzEyLjg1OTcgNS4yMTE5NCAxMi41NDI2IDUuMjM5MzEgMTIuMTc5NCA1LjE0MjAyQzExLjgxMjkgNS4wNDM4NCAxMS41MjkxIDQuNzU2MyAxMS4wODM3IDQuMjcxODJMMTEuMDY5MyA0LjI1NjE2QzEwLjczMzUgMy44OTA5MyAxMC4zMjM5IDMuNDQ1MzggOS43NjM2MiAzLjA5OTcxQzkuMTgyMjcgMi43NDEwMSA4LjQ1ODgzIDIuNSA3LjUwMDA2IDIuNVoiIGZpbGw9IiNmZmZmZmYiPjwvcGF0aD4gPHBhdGggZD0iTTQuMDAwMDYgNi45OTk5OUMyLjk3NDA5IDYuOTk5OTkgMi4wOTIwMyA3LjI3NjkgMS4zOTk2NiA3Ljg3MDM2QzAuNzEyMjcxIDguNDU5NTUgMC4yNjI1OTIgOS4zMTcyNyAwLjAxMzEzNjUgMTAuMzg2NEMtMC4wNDEzMDU3IDEwLjYxOTcgMC4wNzc0MTYyIDEwLjg1ODggMC4yOTYxODYgMTAuOTU2NUMwLjUxNDk1NiAxMS4wNTQyIDAuNzcyMjc2IDEwLjk4MyAwLjkwOTY3MyAxMC43ODY3QzEuMjI2MyAxMC4zMzQ0IDEuNTQyNDQgMTAuMDYyNiAxLjg0NjIgOS45MjMxMkMyLjE0MDM4IDkuNzg4MDQgMi40NTc0NyA5Ljc2MDY3IDIuODIwNjkgOS44NTc5NkMzLjE4NzIzIDkuOTU2MTQgMy40NzA5NyAxMC4yNDM3IDMuOTE2NDMgMTAuNzI4MkwzLjkzMDgyIDEwLjc0MzhDNC4yNjY2IDExLjEwOSA0LjY3NjI0IDExLjU1NDYgNS4yMzY0OSAxMS45MDAzQzUuODE3ODUgMTIuMjU5IDYuNTQxMjggMTIuNSA3LjUwMDA2IDEyLjVDOC41MjYwMiAxMi41IDkuNDA4MDggMTIuMjIzMSAxMC4xMDA1IDExLjYyOTZDMTAuNzg3OCAxMS4wNDA0IDExLjIzNzUgMTAuMTgyNyAxMS40ODcgOS4xMTM2QzExLjU0MTQgOC44ODAyNyAxMS40MjI3IDguNjQxMTMgMTEuMjAzOSA4LjU0MzQzQzEwLjk4NTIgOC40NDU3NCAxMC43Mjc4IDguNTE2OTcgMTAuNTkwNCA4LjcxMzI1QzEwLjI3MzggOS4xNjU1OCA5Ljk1NzY4IDkuNDM3MzYgOS42NTM5MSA5LjU3Njg0QzkuMzU5NzQgOS43MTE5MiA5LjA0MjY0IDkuNzM5MyA4LjY3OTQyIDkuNjQyMDFDOC4zMTI4OSA5LjU0MzgzIDguMDI5MTUgOS4yNTYyOCA3LjU4MzY5IDguNzcxODFMNy41NjkyOSA4Ljc1NjE1QzcuMjMzNTEgOC4zOTA5MiA2LjgyMzg4IDcuOTQ1MzcgNi4yNjM2MiA3LjU5OTY5QzUuNjgyMjcgNy4yNDEgNC45NTg4MyA2Ljk5OTk5IDQuMDAwMDYgNi45OTk5OVoiIGZpbGw9IiNmZmZmZmYiPjwvcGF0aD4gPC9nPjwvc3ZnPg==)
![Vite](https://img.shields.io/badge/%20-Vite-646CFF.svg?style=flat&logo=data:image/svg+xml;base64,PHN2ZyBmaWxsPSIjZmZmZmZmIiB2aWV3Qm94PSItNS42IC01LjYgNjcuMjAgNjcuMjAiIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyI+PGcgaWQ9IlNWR1JlcG9fYmdDYXJyaWVyIiBzdHJva2Utd2lkdGg9IjAiPjwvZz48ZyBpZD0iU1ZHUmVwb190cmFjZXJDYXJyaWVyIiBzdHJva2UtbGluZWNhcD0icm91bmQiIHN0cm9rZS1saW5lam9pbj0icm91bmQiPjwvZz48ZyBpZD0iU1ZHUmVwb19pY29uQ2FycmllciI+PHBhdGggZD0iTSAxMy4xNzU4IDMyLjUwMDAgTCAyNi40MTgwIDMyLjUwMDAgTCAxOS40MzM2IDUxLjQ4NDQgQyAxOC41MTk1IDUzLjg5ODQgMjEuMDI3MyA1NS4xODc1IDIyLjYyMTEgNTMuMjE4NyBMIDQzLjkwMjMgMjYuNTkzOCBDIDQ0LjMwMDggMjYuMTAxNiA0NC41MTE3IDI1LjYzMjggNDQuNTExNyAyNS4wOTM4IEMgNDQuNTExNyAyNC4yMDMxIDQzLjgzMjAgMjMuNTAwMCA0Mi44NDc3IDIzLjUwMDAgTCAyOS41ODIwIDIzLjUwMDAgTCAzNi41ODk5IDQuNTE1NiBDIDM3LjQ4MDQgMi4xMDE2IDM0Ljk5NjEgLjgxMjUgMzMuNDAyMyAyLjgwNDcgTCAxMi4xMjExIDI5LjQwNjMgQyAxMS43MjI2IDI5LjkyMTkgMTEuNDg4MyAzMC4zOTA2IDExLjQ4ODMgMzAuOTA2MyBDIDExLjQ4ODMgMzEuODIwMyAxMi4xOTE0IDMyLjUwMDAgMTMuMTc1OCAzMi41MDAwIFoiPjwvcGF0aD48L2c+PC9zdmc+)
![CMake](https://img.shields.io/badge/CMake-3.20%2B-c0392b?style=flat&logo=cmake&logoColor=white)
![License](https://img.shields.io/badge/License-MIT-475569.svg?logo=data:image/svg+xml;base64,PHN2ZyBmaWxsPSIjZmZmZmZmIiB3aWR0aD0iMTY0cHgiIGhlaWdodD0iMTY0cHgiIHZpZXdCb3g9IjAgMCA1MTIuMDAgNTEyLjAwIiB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHN0cm9rZT0iI2ZmZmZmZiIgc3Ryb2tlLXdpZHRoPSIwLjAwNTEyIj48ZyBpZD0iU1ZHUmVwb19iZ0NhcnJpZXIiIHN0cm9rZS13aWR0aD0iMCI+PC9nPjxnIGlkPSJTVkdSZXBvX3RyYWNlckNhcnJpZXIiIHN0cm9rZS1saW5lY2FwPSJyb3VuZCIgc3Ryb2tlLWxpbmVqb2luPSJyb3VuZCIgc3Ryb2tlPSIjQ0NDQ0NDIiBzdHJva2Utd2lkdGg9IjMuMDcyIj48L2c+PGcgaWQ9IlNWR1JlcG9faWNvbkNhcnJpZXIiPjxwYXRoIGQ9Ik0yNTYgOEMxMTkuMDMzIDggOCAxMTkuMDMzIDggMjU2czExMS4wMzMgMjQ4IDI0OCAyNDggMjQ4LTExMS4wMzMgMjQ4LTI0OFMzOTIuOTY3IDggMjU2IDh6bTExNy4xMzQgMzQ2Ljc1M2MtMS41OTIgMS44NjctMzkuNzc2IDQ1LjczMS0xMDkuODUxIDQ1LjczMS04NC42OTIgMC0xNDQuNDg0LTYzLjI2LTE0NC40ODQtMTQ1LjU2NyAwLTgxLjMwMyA2Mi4wMDQtMTQzLjQwMSAxNDMuNzYyLTE0My40MDEgNjYuOTU3IDAgMTAxLjk2NSAzNy4zMTUgMTAzLjQyMiAzOC45MDRhMTIgMTIgMCAwIDEgMS4yMzggMTQuNjIzbC0yMi4zOCAzNC42NTVjLTQuMDQ5IDYuMjY3LTEyLjc3NCA3LjM1MS0xOC4yMzQgMi4yOTUtLjIzMy0uMjE0LTI2LjUyOS0yMy44OC02MS44OC0yMy44OC00Ni4xMTYgMC03My45MTYgMzMuNTc1LTczLjkxNiA3Ni4wODIgMCAzOS42MDIgMjUuNTE0IDc5LjY5MiA3NC4yNzcgNzkuNjkyIDM4LjY5NyAwIDY1LjI4LTI4LjMzOCA2NS41NDQtMjguNjI1IDUuMTMyLTUuNTY1IDE0LjA1OS01LjAzMyAxOC41MDggMS4wNTNsMjQuNTQ3IDMzLjU3MmExMi4wMDEgMTIuMDAxIDAgMCAxLS41NTMgMTQuODY2eiI+PC9wYXRoPjwvZz48L3N2Zz4=)
<br/>
[![Quality Gate Status](https://sonarcloud.io/api/project_badges/measure?project=lextpf_salma&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=lextpf_salma)
[![Maintainability Rating](https://sonarcloud.io/api/project_badges/measure?project=lextpf_salma&metric=sqale_rating)](https://sonarcloud.io/summary/new_code?id=lextpf_salma)
[![Reliability Rating](https://sonarcloud.io/api/project_badges/measure?project=lextpf_salma&metric=reliability_rating)](https://sonarcloud.io/summary/new_code?id=lextpf_salma)
<br/>
[![build](https://github.com/lextpf/salma/actions/workflows/build.yml/badge.svg)](https://github.com/lextpf/salma/actions/workflows/build.yml)
[![tests](https://github.com/lextpf/salma/actions/workflows/test.yml/badge.svg)](https://github.com/lextpf/salma/actions/workflows/test.yml)
<br/>
![Sponsor](https://img.shields.io/static/v1?label=sponsor&message=%E2%9D%A4&color=ff69b4&logo=data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCA2NDAgNjQwIj48IS0tIUZvbnQgQXdlc29tZSBQcm8gdjcuMi4wIGJ5IEBmb250YXdlc29tZSAtIGh0dHBzOi8vZm9udGF3ZXNvbWUuY29tIExpY2Vuc2UgLSBodHRwczovL2ZvbnRhd2Vzb21lLmNvbS9saWNlbnNlIChDb21tZXJjaWFsIExpY2Vuc2UpIENvcHlyaWdodCAyMDI2IEZvbnRpY29ucywgSW5jLi0tPjxwYXRoIG9wYWNpdHk9IjEiIGZpbGw9IiNmZjY5YjRmZiIgZD0iTTMyIDQ4MEwzMiA1NDRDMzIgNTYxLjcgNDYuMyA1NzYgNjQgNTc2TDM4NC41IDU3NkM0MTMuNSA1NzYgNDQxLjggNTY2LjcgNDY1LjIgNTQ5LjVMNTkxLjggNDU2LjJDNjA5LjYgNDQzLjEgNjEzLjQgNDE4LjEgNjAwLjMgNDAwLjNDNTg3LjIgMzgyLjUgNTYyLjIgMzc4LjcgNTQ0LjQgMzkxLjhMNDI0LjYgNDgwTDMxMiA0ODBDMjk4LjcgNDgwIDI4OCA0NjkuMyAyODggNDU2QzI4OCA0NDIuNyAyOTguNyA0MzIgMzEyIDQzMkwzODQgNDMyQzQwMS43IDQzMiA0MTYgNDE3LjcgNDE2IDQwMEM0MTYgMzgyLjMgNDAxLjcgMzY4IDM4NCAzNjhMMjMxLjggMzY4QzE5Ny45IDM2OCAxNjUuMyAzODEuNSAxNDEuMyA0MDUuNUw5OC43IDQ0OEw2NCA0NDhDNDYuMyA0NDggMzIgNDYyLjMgMzIgNDgweiIvPjxwYXRoIGZpbGw9InJnYmEoMjU1LCAyNTUsIDI1NSwgMS4wMCkiIGQ9Ik0yNTAuOSA2NEMyNzQuOSA2NCAyOTcuNSA3NS41IDMxMS42IDk1TDMyMCAxMDYuN0wzMjguNCA5NUMzNDIuNSA3NS41IDM2NS4xIDY0IDM4OS4xIDY0QzQzMC41IDY0IDQ2NCA5Ny41IDQ2NCAxMzguOUw0NjQgMTQxLjNDNDY0IDIwNS43IDM4MiAyNzQuNyAzNDEuOCAzMDQuNkMzMjguOCAzMTQuMyAzMTEuMyAzMTQuMyAyOTguMyAzMDQuNkMyNTguMSAyNzQuNiAxNzYgMjA1LjcgMTc2LjEgMTQxLjNMMTc2LjEgMTM4LjlDMTc2IDk3LjUgMjA5LjUgNjQgMjUwLjkgNjR6Ii8+PC9zdmc+)
</div>

**A mod installer** built with **C++23** and powered by *Crow* and *React*. It pairs a **C DLL** that handles archive extraction and FOMOD processing with a **Crow HTTP server** and **React web interface** for interactive installations. salma integrates with **Mod Organizer 2** via a Python plugin, automatically **inferring FOMOD selections** by comparing archives against installed files - no wizard clicks required.

<div align="center">
<br>

<img src="PREVIEW.png" alt="Preview" width="600"/>

</div>

> [!IMPORTANT]
> **Early release** - salma is under active development and has been tested against:
> - **Nolvus Ascension 6.0.20** with **350 FOMODs**.
> - Other mods should work without issues - if you run into a problem, please report it.

```
/* ============================================================================================== *
 *
 *       ::::::::      :::     :::        ::::    ::::      :::         ⢠⣤⣤⣀ ⠀⠀⠀⠀⠀⠀ ⣀⣤⣤⡄
 *      :+:    :+:   :+: :+:   :+:        +:+:+: :+:+:+   :+: :+:      ⢸⣿⣿⣿⣿⣦⣄⣀⣠⣴⣿⣿⣿⣿⡇⠀⊹
 *      +:+         +:+   +:+  +:+        +:+ +:+:+ +:+  +:+   +:+     ⣸⣿⣿⣿⣿⣿⡽⣿⣯⣿⣿⣿⣿⣿⣇
 *      +#++:++#++ +#++:++#++: +#+        +#+  +:+  +#+ +#++:++#++:    ⢻⣿⣿⣿⠿⣻⣵⡟⣮⣟⠿⣿⣿⣿⡟
 *             +#+ +#+     +#+ +#+        +#+       +#+ +#+     +#+    ⠀⠀⠀⠀⣼⣿⡿ ⠀⢿⣿⣷⡀
 *      #+#    #+# #+#     #+# #+#        #+#       #+# #+#     #+#    ⊹⠀⣠⣾⣿⣿⠃ ⠀⠈⢿⣿⣿⣦⡀
 *       ########  ###     ### ########## ###       ### ###     ###    ⠀⠈⠉⠹⡿⠁⠀⠀⠀⠀⠈⢻⡇⠉⠉
 *
 *                                 << M O D   I N S T A L L E R >>
 *
 * ============================================================================================== */
```

## Features

### Interface

salma ships with a **C DLL** for direct integration, a **REST API** for programmatic access, and a **React web UI** for interactive installations.

```mermaid
---
config:
  look: handDrawn
  theme: mc
  themeVariables:
    fontSize: 18px
  layout: elk
---
graph LR
    classDef dll fill:#134e3a,stroke:#10b981,color:#e2e8f0
    classDef api fill:#1e3a5f,stroke:#3b82f6,color:#e2e8f0
    classDef web fill:#2e1f5e,stroke:#8b5cf6,color:#e2e8f0

    subgraph DLL["📦 C DLL (mo2-salma.dll)"]
        Install["⚙️ install()"]:::dll
        Infer["🧠 inferFomodSelections()"]:::dll
        Config["🔧 installWithConfig()"]:::dll
    end

    subgraph API["🌐 Crow REST API"]
        Upload["📤 Upload & Install"]:::api
        Scan["🔍 FOMOD Scan"]:::api
        Status["📊 Job Status"]:::api
    end

    subgraph Web["🖥️ React Frontend"]
        UI["🎮 Interactive Installer"]:::web
        FomodView["📂 FOMOD Browser"]:::web
        Logs["📋 Log Viewer"]:::web
    end

    Web --> API
    API --> DLL
```

### FOMOD Processing

```mermaid
---
config:
  look: handDrawn
  theme: mc
  themeVariables:
    fontSize: 18px
  layout: elk
---
graph LR
    classDef parse fill:#7c2d12,stroke:#f97316,color:#fef3c7
    classDef eval fill:#4c1d95,stroke:#e879f9,color:#e2e8f0
    classDef ops fill:#064e3b,stroke:#34d399,color:#e2e8f0
    classDef detect fill:#713f12,stroke:#facc15,color:#fef9c3

    P["📄 XML Parser"]:::parse
    D["🧩 Dependency Evaluator"]:::eval
    F["📂 File Operations"]:::ops
    S["🔎 Structure Detector"]:::detect

    P --- D --- F --- S
```

- 📄 **XML Parser** - Parses `fomod/ModuleConfig.xml` for installation steps and options
- 🧩 **Dependency Evaluator** - Resolves flag-based and file-based FOMOD dependencies
- 📂 **File Operations** - Priority-sorted file copy, folder creation, and patching
- 🔎 **Structure Detector** - Identifies mod folder layout (meshes/, textures/, SKSE/, etc.)

### Inference Engine

salma's inference service compares an archive's FOMOD options against an already-installed mod to determine which selections were originally chosen:

- 🌳 Walks every permutation of FOMOD steps and flags
- 🎯 Matches expected file output against the installed file tree
- 📋 Returns a JSON configuration that reproduces the original install
- 🧪 Powers the round-trip test suite for validation

### MO2 Integration

- 🐍 **Python Plugin** - `mo2-salma.py` loads the DLL via ctypes and exposes tools inside MO2
- 🔬 **Scan FOMOD Choices** - Batch-scans all installed mods for FOMOD selections
- 📁 **Centralized Output** - Reinstalled mods go to a dedicated "Salma FOMODs Output" folder
- ⚡ **Deploy & Purge** - `deploy.bat` and `purge.bat` scripts for plugin lifecycle management

### Archive Support

|  Format | Backend                                             |
|---------|-----------------------------------------------------|
| 7z      | bit7z (native 7-Zip SDK wrapper)                    |
| ZIP     | libarchive                                          |
| RAR     | libarchive                                          |
| TAR.*   | libarchive (bzip2, lz4, lzma, zstd)                |

### Additional Capabilities

- 🌐 **REST API** - Full programmatic access for custom installers and automation
- 🎨 **Web UI** - React SPA with dark/light theme, served directly by the Crow backend
- 📝 **Logging** - Unified thread-safe logger with subsystem tags (`[archive]` `[install]` `[fomod]` `[infer]` `[server]`)
- 🧪 **Round-Trip Testing** - Infer selections, reinstall, and diff against the original mod

## Technology Stack

| Component       | Technology                     |
|-----------------|--------------------------------|
| Language        | C++23                          |
| HTTP Framework  | Crow (with CORS handler)       |
| Frontend        | React 18 + TypeScript + Vite   |
| Styling         | Tailwind CSS 4                 |
| XML Parsing     | pugixml                        |
| JSON            | nlohmann-json                  |
| Archive         | libarchive + bit7z             |
| Formatting      | clang-format (Google-based)    |
| Build System    | CMake 3.20+                    |
| Package Manager | vcpkg                          |
| Documentation   | Doxide + MkDocs                |
| Plugin          | Python 3 (MO2 ctypes bridge)   |
| CI/CD           | GitHub Actions                 |
| Platform        | Windows 10/11 (64-bit)         |

## Quick Start

### Prerequisites

- **Windows 10/11** (64-bit)
- **Visual Studio 2022** (MSVC v143, C++23)
- **CMake 3.20+**
- **vcpkg** with the toolchain at a known path
- **Node.js** (for building the React frontend)
- **Python 3** (for MO2 plugin and documentation post-processing)
- **clang-format** (optional, for code formatting - CI enforces it)
- **doxide** + **mkdocs** (optional, for API docs generation)

### Building

```powershell
# 1. Clone the repository
git clone https://github.com/lextpf/salma.git
cd salma

# 2. Build (format + configure + compile + docs)
.\build.bat

# 3. Run the server
.\build\bin\Release\mo2-server.exe
```

Output:
- DLL: `build/bin/Release/mo2-salma.dll`
- Server: `build/bin/Release/mo2-server.exe`

### Deploying to MO2

```powershell
# Copy DLL + Python plugin to your MO2 instance
.\deploy.bat
```

## Architecture

salma ships **three deployment artifacts** that share **one core library**. The library (`mo2-core`) holds all the actual installation, FOMOD parsing, and inference logic. The artifacts are different *shells* around that library - one for MO2 (a DLL), one for HTTP (an EXE), and one for the browser (a React SPA served by the EXE). You can use any subset depending on how you want to drive salma.

```mermaid
---
config:
  look: handDrawn
  theme: mc
  themeVariables:
    fontSize: 18px
  layout: elk
---
graph TB
    classDef host fill:#1e1e2e,stroke:#94a3b8,color:#e2e8f0,stroke-dasharray:6 4
    classDef artifact fill:#1e3a5f,stroke:#3b82f6,color:#e2e8f0
    classDef core fill:#134e3a,stroke:#10b981,color:#e2e8f0

    subgraph MO2["🧩 Mod Organizer 2 process"]
        Plugin["🐍 mo2-salma.py (Python plugin)"]:::host
        DLL["📦 mo2-salma.dll (C-API exports)"]:::artifact
        Plugin -- ctypes --> DLL
    end

    subgraph SrvHost["🖥️ mo2-server.exe process (port 5000)"]
        Server["🌐 Crow HTTP server + REST endpoints"]:::artifact
        Static["📄 Static file handler"]:::artifact
        Server --- Static
    end

    subgraph Browser["🌍 Web browser"]
        SPA["⚛️ React SPA (web/dist)"]:::artifact
    end

    Core["🛠️ mo2-core (shared C++ library)<br/>extraction · FOMOD parser · inference · file ops · logger"]:::core

    DLL --> Core
    Server --> Core
    SPA -- "HTTP /api" --> Server
    Static -- "serves" --> SPA
```

### The three artifacts

- **📦 `mo2-salma.dll` - the headless library.** Built from the `mo2-core` CMake target. Has no HTTP, no UI, no Crow dependency. Mod Organizer 2 loads it through `mo2-salma.py` (a Python plugin) using `ctypes` and calls C-linkage exports like `install()`, `inferFomodSelections()`, and `installWithConfig()`. **Use this when** you want MO2 to install FOMOD-aware mods without the wizard.

- **🌐 `mo2-server.exe` - the HTTP shell.** Built from the `mo2-server` CMake target. Links the same `mo2-core` library that the DLL exports, **plus** the Crow HTTP framework on top. Exposes the install / infer / scan / status / log endpoints under `/api/*` and serves the React SPA from `web/dist/` at `/`. Listens on `:5000`. **Use this when** you want a graphical interface or programmatic REST access.

- **⚛️ React SPA (`web/dist/`) - the browser front-end.** Built with Vite from `web/src/`. Pure HTML/CSS/JS - knows nothing about C++. Talks to `mo2-server.exe` over `fetch('/api/...')`. **Use this when** you want to drive an installation interactively, browse FOMOD JSON files, or watch logs in real time.

> [!NOTE]
> The DLL and the EXE are different binaries built from the same source tree. They share `mo2-core` so that improving FOMOD inference once benefits both the MO2 plugin and the web UI - there is no duplicated logic to keep in sync.

### Two ways salma is used

**Path A - inside Mod Organizer 2 (no server, no browser).**

1. `deploy.bat` copies `mo2-salma.dll` and `mo2-salma.py` into `<MO2 instance>/plugins/`.
2. MO2 starts and Python loads `mo2-salma.py`.
3. The plugin loads the DLL via `ctypes.CDLL("mo2-salma.dll")`.
4. When the user installs a mod, the plugin calls `install()` (or `inferFomodSelections()` for batch scans) directly into `mo2-core`.
5. Results return as JSON or path strings; the plugin renders them in MO2's tools menu.

**Path B - the standalone server with the web UI.**

1. The user runs `mo2-server.exe` (port 5000).
2. The browser opens `http://localhost:5000` and the server's static handler returns `web/dist/index.html`.
3. The React SPA loads, then calls endpoints like `POST /api/installation/upload` for file uploads or `GET /api/mo2/fomods/scan/status` for background-job progress.
4. The Crow controllers in `mo2-server` translate those requests into calls into `mo2-core` (the same calls the DLL exports).
5. Results come back as JSON; the SPA renders progress, logs, and FOMOD trees.

Both paths converge in `mo2-core`. The DLL exposes it as a flat C ABI; the server exposes it as REST endpoints.

### File-to-purpose drill-down

|                       File | Purpose                                          |
|----------------------------|--------------------------------------------------|
|                 `main.cpp` | Crow HTTP server entry point                     |
|     `InstallationService`  | Main orchestrator for mod installation           |
|         `ArchiveService`   | Archive extraction (libarchive + bit7z)          |
|           `FomodService`   | FOMOD XML parsing and installation logic         |
|   `FomodInferenceService`  | Infers FOMOD selections from installed files     |
| `FomodDependencyEvaluator` | Evaluates FOMOD flag and file dependencies       |
|        `FileOperations`    | Priority-sorted file copy and patching           |
|     `ModStructureDetector` | Detects mod folder layout                        |
|                  `CApi`    | C-linkage DLL exports for ctypes                 |
|                `Logger`    | Thread-safe logging with callback support        |
|         `ConfigService`    | Configuration management                         |

## Project Structure

```
salma/
|-- src/                               # C++ source code
|   |-- main.cpp                       # Crow HTTP server entry point
|   |-- CApi.h/cpp                     # C-linkage DLL API (ctypes)
|   |-- Export.h                       # MO2_API export macro
|   |-- Types.h                        # Shared type definitions
|   |-- Utils.h/cpp                    # Shared string/path helpers
|   |-- BackgroundJob.h                # Async job runner
|   |-- Logger.h/cpp                   # Thread-safe logging
|   |-- ArchiveService.h/cpp           # Archive extraction
|   |-- FileOperations.h/cpp           # Queued file operations
|   |-- ModStructureDetector.h/cpp     # Mod folder structure detection
|   |-- FomodService.h/cpp             # FOMOD installation logic
|   |-- FomodDependencyEvaluator.h/cpp # FOMOD dependency evaluation
|   |-- FomodInferenceService.h/cpp    # Selection inference engine (orchestrator)
|   |-- FomodIR{,Parser}.h/cpp         # FOMOD intermediate representation + XML parser
|   |-- FomodCSP*.h/cpp                # CSP solver, options, precompute, types
|   |-- FomodPropagator.h/cpp          # Constraint propagator
|   |-- FomodForwardSimulator.h/cpp    # Forward-simulates installs against the IR
|   |-- FomodInferenceAtoms.h/cpp      # Atom-level inference helpers
|   |-- FomodAtom.h                    # Atom type definitions
|   |-- InstallationService.h/cpp      # Main orchestrator
|   |-- InstallationController.h/cpp   # REST endpoint handlers
|   |-- Mo2Controller.h/cpp            # MO2 dashboard controller (shared state)
|   |-- Mo2{Config,Fomod,Log,Plugin,Test}Controller.cpp # Per-subsystem endpoints
|   |-- Mo2Helpers.h/cpp               # Shared helpers for MO2 controllers
|   |-- ConfigService.h/cpp            # Configuration management
|   |-- MultipartHandler.h/cpp         # Form data parsing
|   +-- StaticFileHandler.h/cpp        # SPA serving
|-- web/                               # React frontend
|   |-- src/                           # TypeScript source
|   |-- dist/                          # Built SPA (served by Crow)
|   |-- package.json                   # Dependencies
|   +-- vite.config.ts                 # Dev proxy to :5000
|-- tests/                             # GoogleTest C++ unit tests
|-- scripts/                           # MO2 plugin & utilities
|   |-- mo2-salma.py                   # MO2 Python plugin
|   |-- common.py                      # Shared utilities
|   +-- _clean_docs.py                 # Doc post-processing
|-- logs/                              # Runtime logs
|   +-- salma.log                      # Application log
|-- .clang-format                      # Code formatting rules
|-- CMakeLists.txt                     # Build configuration (mo2-core + mo2-server targets)
|-- CMakePresets.json                  # Build presets (vcpkg)
|-- vcpkg.json                         # Dependency manifest
|-- build.bat                          # Build pipeline
|-- deploy.bat                         # Deploy to MO2
|-- purge.bat                          # Remove plugin & clean output
|-- test.bat                           # Run C++ unit tests (salma_tests.exe)
|-- test.py                            # Round-trip test runner (Python)
|-- test_one.py                        # Round-trip test for a single mod
|-- test.log                           # Round-trip test output
|-- doxide.yml                         # API doc config
+-- mkdocs.yml                         # Documentation site config
```

## Documentation

API documentation is generated via a three-stage pipeline:

```powershell
# 1. Generate markdown from C++ headers
doxide build

# 2. Post-process (strip noise, fix formatting)
python scripts/_clean_docs.py

# 3. Build the documentation site
mkdocs build
```

The site is output to `site/` and can be served locally with `mkdocs serve`.

## Troubleshooting

|                  Problem | Solution                                                                |
|--------------------------|-------------------------------------------------------------------------|
| vcpkg toolchain not found | Set `VCPKG_ROOT` or update the path in `CMakePresets.json`             |
| Crow port already in use | Another process is on port 5000, kill it or change the port             |
| DLL not found by plugin  | Run `deploy.bat` or manually copy `mo2-salma.dll` to MO2 plugins       |
| FOMOD inference mismatch | Mod may have been manually edited post-install, check `.mohidden` files |
| Vite proxy errors        | Ensure the Crow server is running on port 5000 before starting Vite     |

## Contributing

Contributions are welcome! Please read the [Contributing Guidelines](CONTRIBUTING.md) before submitting pull requests.

### Development Workflow

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. ~~Run tests and~~ ensure the build passes
5. Commit with descriptive messages
6. Push to your fork and open a Pull Request

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- [Crow](https://crowcpp.org/) - C++ HTTP framework
- [React](https://react.dev/) - Frontend UI library
- [libarchive](https://www.libarchive.org/) - Multi-format archive extraction
- [bit7z](https://github.com/rikyoz/bit7z) - 7-Zip SDK wrapper
- [pugixml](https://pugixml.org/) - XML parsing
- [nlohmann-json](https://github.com/nlohmann/json) - JSON for Modern C++
- [vcpkg](https://github.com/microsoft/vcpkg) - C++ package manager
- [Doxide](https://github.com/lawmurray/doxide) - API documentation generator
- [MkDocs Material](https://squidfunk.github.io/mkdocs-material/) - Documentation theme
- [Tailwind CSS](https://tailwindcss.com/) - Utility-first CSS framework
- [Claude](https://claude.ai/) - AI coding assistant by Anthropic
- [Codex](https://openai.com/index/openai-codex/) - AI coding assistant by OpenAI
