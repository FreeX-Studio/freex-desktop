// Copyright (c) 2026 MirraNET, Niklas Linz. Licensed under the GNU LGPLv3.
//
// Jenkins build of FreeX Desktop on a Linux agent: cross-compiles the Windows app and packs the
// installer (see windows/build.sh for the tools the agent needs).
pipeline {
    agent any
    options {
        buildDiscarder(logRotator(numToKeepStr: '20'))
        timeout(time: 60, unit: 'MINUTES')
    }
    stages {
        stage('Build') {
            steps {
                sh 'windows/build.sh'
            }
        }
    }
    post {
        success {
            archiveArtifacts artifacts: 'dist/freex-desktop-*-setup.exe, dist/freex-desktop.exe, dist/WebView2Loader.dll', fingerprint: true
        }
    }
}
