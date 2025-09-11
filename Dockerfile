FROM ubuntu:24.04

ARG DEBIAN_FRONTEND=noninteractive
ARG SDK_ASSET_NAME="stm32mp1-kaonic-proto-sdk-aarch64.sh"

RUN apt-get update && apt-get install -y --no-install-recommends \ 
    ca-certificates \
    build-essential \
    make \
    curl \
    git \
    python3 \
    wget \
    gcc-arm-linux-gnueabihf \
    g++-arm-linux-gnueabihf \
    gcc-aarch64-linux-gnu \
    g++-aarch64-linux-gnu \
    ninja-build \
    neovim \
    openssh-client \
    rsync \
    file \
    libclang-dev \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /opt/

# Download latest Yocto SDK
# RUN set -e && DOWNLOAD_URL=$(curl -s https://api.github.com/repos/BeechatNetworkSystemsLtd/kaonic-yocto/releases/latest \
#     | grep "browser_download_url" \
#     | grep "${SDK_ASSET_NAME}" \
#     | cut -d '"' -f 4) \
#     && wget "$DOWNLOAD_URL" -O /opt/yocto-sdk.sh

# Install yocoto sdk
# RUN chmod +x /opt/yocto-sdk.sh \
#     && /opt/yocto-sdk.sh -y -d /opt/yocto-sdk

CMD ["/bin/bash"]

