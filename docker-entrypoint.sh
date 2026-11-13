#!/bin/sh
# 컨테이너가 처음 켜질 때 딱 한 번, 서버를 띄우기 전에 실행되는 스크립트.
#
# [배경] 이 이미지는 서비스를 root로 돌리지 않는다(appuser). 그런데 호스트 디렉토리를
# bind mount로 붙이면 그 디렉토리의 소유권은 호스트 쪽 값이 그대로 보이므로, 이미지를
# 만들 때 미리 chown 해둔 것은 마운트에 덮여 무의미해진다. 그래서 컨테이너는 root로
# 시작해 마운트된 뒤의 실제 디렉토리를 정리하고, 그 다음에 appuser로 내려앉는다.
# postgres 같은 공식 이미지들이 쓰는 것과 같은 방식이다 — 덕분에 이 이미지를 쓰는
# 쪽은 호스트에서 mkdir이나 chown을 미리 칠 필요가 없다.
set -eu

: "${DATA_DIR:=/var/lib/app}"
APP_USER=appuser

# compose의 `user:`로 uid를 직접 지정했거나 쿠버네티스에서 runAsNonRoot로 띄운 경우엔 여기가
# root가 아니다. 그때는 애초에 chown도 사본 만들기도 할 권한이 없고, 띄운 쪽이 데이터 폴더
# 소유권과 /app/.env의 읽기 권한을 책임진다는 뜻이므로 그냥 넘긴다(앱이 /app/.env를 직접 읽는다).
if [ "$(id -u)" = "0" ]; then
    mkdir -p "$DATA_DIR"

    # [설명] 루트 디렉토리의 소유자만 보고 판단한다. 파일이 수십만 개 쌓인 뒤에도
    # 재시작할 때마다 전체를 순회하면 기동이 눈에 띄게 느려지기 때문이다. 이미 우리
    # 것이면 하위도 우리 것이라고 보는 낙관적인 가정이며, 밖에서 파일을 직접 밀어넣어
    # 소유자가 섞였다면 한 번은 손으로 맞춰줘야 한다.
    if [ "$(stat -c %u "$DATA_DIR")" != "$(id -u "$APP_USER")" ]; then
        chown -R "$APP_USER:$APP_USER" "$DATA_DIR"
    fi

    # [설명] 마운트된 설정 파일(/app/.env)을 root 권한으로 읽어 appuser만 읽을 수 있는 사본을
    # 만들고, 작업 디렉토리를 그쪽으로 옮긴다. 앱은 "작업 디렉토리의 .env"를 읽으므로 코드는
    # 그대로다. 덕분에 호스트의 .env가 600이든 소유자가 누구든 신경 쓸 필요가 없다 — root로
    # 시작하는 다른 이미지들이 설정을 먼저 읽고 권한을 내리는 것과 같은 순서다.
    #
    # [설명] 원본 마운트를 그대로 chown/chmod하지 않는 이유: 읽기 전용(:ro)으로 붙어 있고,
    # 무엇보다 호스트 파일의 권한을 컨테이너가 바꿔 놓으면 안 된다.
    #
    # [주의] 호스트에 .env가 없으면 도커가 그 자리에 **빈 디렉토리**를 만들어 붙인다. 그대로
    # 두면 앱이 "설정이 없다"는 엉뚱한 에러로만 죽으므로, 여기서 원인을 직접 말하고 멈춘다.
    # 파일이 아예 없는 경우(마운트하지 않은 구성)는 그냥 넘어간다.
    ENV_FILE=/app/.env
    RUN_DIR=/run/app
    if [ -e "$ENV_FILE" ]; then
        if [ ! -f "$ENV_FILE" ]; then
            echo "error: $ENV_FILE is not a regular file." >&2
            echo "       did the host .env exist when the container was created? (cp .env.example .env)" >&2
            exit 1
        fi
        mkdir -p "$RUN_DIR"
        install -m 400 -o "$APP_USER" -g "$APP_USER" "$ENV_FILE" "$RUN_DIR/.env"
        cd "$RUN_DIR"
    fi

    # [설명] gosu 대신 setpriv를 쓴다 — util-linux에 들어 있어 debian 베이스에 이미
    # 있으므로, "apt로 아무것도 설치하지 않는다"는 이 이미지의 성질을 깨지 않는다.
    # --clear-groups가 없으면 root의 보조 그룹이 그대로 남는다.
    #
    # [주의] uid/gid를 이름이 아니라 숫자로 넘긴다. setpriv 버전에 따라 이름을
    # 받아주지 않는 경우가 있다.
    exec setpriv \
        --reuid "$(id -u "$APP_USER")" \
        --regid "$(id -g "$APP_USER")" \
        --clear-groups \
        "$@"
fi

# [설명] exec이라 이 스크립트의 프로세스를 서버가 그대로 물려받는다(PID 1). 그래야
# 도커가 보내는 SIGTERM이 셸이 아니라 서버에 직접 닿아서 graceful shutdown이 동작한다.
exec "$@"
