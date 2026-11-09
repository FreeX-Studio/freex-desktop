// Copyright (c) 2026 MirraNET, Niklas Linz. Licensed under the GNU LGPLv3.
//
// Jenkins build of FreeX Desktop on a Linux agent: checks and tests, the Linux app with its
// Debian package, and the Windows app (cross-compiled) with its installer.
//
// The agent needs, besides rustup and the Windows tools listed in packaging/windows/build.sh:
//
//   sudo apt install build-essential pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev \
//       libayatana-appindicator3-dev librsvg2-dev librsvg2-bin dpkg \
//       gcc-mingw-w64-x86-64 nsis curl unzip
//
// Artifacts in dist/: the .deb, the setup .exe and the plain Windows app.
pipeline {
    agent any

    options {
        buildDiscarder(logRotator(numToKeepStr: '20', artifactNumToKeepStr: '10'))
        disableConcurrentBuilds()
        timeout(time: 60, unit: 'MINUTES')
    }

    environment {
        PATH = "${env.HOME}/.cargo/bin:${env.PATH}"
        CARGO_TERM_COLOR = 'never'
    }

    stages {
        stage('Check') {
            steps {
                // The workspace is kept between builds; packages of earlier versions must go.
                sh 'rm -rf dist'
                dir('src-tauri') {
                    sh 'cargo fmt --check'
                    sh 'cargo clippy --locked --all-targets -- -D warnings'
                }
            }
        }

        stage('Test') {
            steps {
                dir('src-tauri') {
                    sh 'cargo test --locked'
                }
            }
        }

        stage('Linux') {
            steps {
                dir('src-tauri') {
                    sh 'cargo build --release --locked --features custom-protocol'
                }
                sh '''
                    VERSION="$(sed -n 's/^version *= *"\\(.*\\)"/\\1/p' src-tauri/Cargo.toml | head -n 1)"
                    packaging/deb/build-deb.sh "$VERSION" src-tauri/target/release/freex-desktop dist
                '''
            }
        }

        stage('Windows') {
            steps {
                sh 'packaging/windows/build.sh'
            }
        }
    }

    post {
        success {
            archiveArtifacts artifacts: 'dist/*.deb, dist/freex-desktop-*-setup.exe, dist/freex-desktop.exe, dist/WebView2Loader.dll', fingerprint: true
        }
    }
}
