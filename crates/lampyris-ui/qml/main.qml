// Lampyris — main window (Qt Quick + KDE Kirigami)
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as Controls
import org.kde.kirigami as Kirigami

import org.bogd3v.lampyris

Kirigami.ApplicationWindow {
    id: root
    title: qsTr("Lampyris")
    minimumWidth: Kirigami.Units.gridUnit * 24
    width: Kirigami.Units.gridUnit * 48
    height: Kirigami.Units.gridUnit * 36

    LampyrisController {
        id: controller
    }

    readonly property var clusters: {
        try {
            return JSON.parse(controller.clusters_json)
        } catch (e) {
            return []
        }
    }

    pageStack.initialPage: Kirigami.ScrollablePage {
        id: page
        title: qsTr("Coordinated activity")

        actions: [
            Kirigami.Action {
                text: controller.running ? qsTr("Stop") : qsTr("Watch Bluesky")
                icon.name: controller.running ? "media-playback-stop" : "media-playback-start"
                onTriggered: controller.running
                    ? controller.stop()
                    : controller.start(langField.text, minAccounts.value)
            }
        ]

        header: ColumnLayout {
            spacing: 0

            Kirigami.InlineMessage {
                Layout.fillWidth: true
                visible: true
                position: Kirigami.InlineMessage.Position.Header
                type: controller.running ? Kirigami.MessageType.Positive : Kirigami.MessageType.Information
                text: controller.status
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.margins: Kirigami.Units.largeSpacing
                spacing: Kirigami.Units.largeSpacing

                Controls.Label { text: qsTr("Languages") }
                Controls.TextField {
                    id: langField
                    Layout.preferredWidth: Kirigami.Units.gridUnit * 8
                    placeholderText: qsTr("all (e.g. es, pt)")
                    enabled: !controller.running
                }
                Controls.Label { text: qsTr("Min. accounts") }
                Controls.SpinBox {
                    id: minAccounts
                    from: 2
                    to: 1000
                    value: 5
                    enabled: !controller.running
                }
                Item { Layout.fillWidth: true }
                Controls.Label {
                    text: qsTr("%1 posts · %2 clusters")
                        .arg(controller.posts_seen.toLocaleString(Qt.locale(), "f", 0))
                        .arg(controller.clusters_reported)
                    color: Kirigami.Theme.disabledTextColor
                }
            }
        }

        ListView {
            id: list
            model: root.clusters
            spacing: Kirigami.Units.largeSpacing
            topMargin: Kirigami.Units.largeSpacing
            bottomMargin: Kirigami.Units.largeSpacing

            Kirigami.PlaceholderMessage {
                anchors.centerIn: parent
                width: parent.width - Kirigami.Units.gridUnit * 4
                visible: list.count === 0
                icon.name: "view-visible"
                text: controller.running ? qsTr("Listening… nothing coordinated yet")
                                         : qsTr("Press “Watch Bluesky” to start")
                explanation: qsTr("Lampyris reports groups of distinct accounts posting almost the same text within minutes. A cluster is a pattern to look into, not proof of bots.")
            }

            delegate: Kirigami.AbstractCard {
                id: card
                required property var modelData
                width: ListView.view.width - Kirigami.Units.largeSpacing * 2
                x: Kirigami.Units.largeSpacing

                contentItem: ColumnLayout {
                    spacing: Kirigami.Units.smallSpacing

                    RowLayout {
                        Layout.fillWidth: true
                        Kirigami.Heading {
                            level: 3
                            text: qsTr("%1 accounts · %2 posts").arg(card.modelData.accounts).arg(card.modelData.posts)
                        }
                        Controls.Label {
                            visible: card.modelData.closed
                            text: qsTr("closed")
                            color: Kirigami.Theme.disabledTextColor
                        }
                        Item { Layout.fillWidth: true }
                        Controls.Label {
                            text: qsTr("score %1").arg(card.modelData.score.toFixed(2))
                            font.bold: true
                            color: card.modelData.score > 0.8 ? Kirigami.Theme.negativeTextColor
                                 : card.modelData.score > 0.5 ? Kirigami.Theme.neutralTextColor
                                 : Kirigami.Theme.textColor
                        }
                    }

                    Controls.ProgressBar {
                        Layout.fillWidth: true
                        from: 0
                        to: 1
                        value: card.modelData.score
                    }

                    Controls.Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        text: "“" + card.modelData.text + "”"
                        font.italic: true
                    }

                    Controls.Label {
                        Layout.fillWidth: true
                        wrapMode: Text.Wrap
                        color: Kirigami.Theme.disabledTextColor
                        text: qsTr("%1 – %2 · %3 min · %4 accounts/min · model %5")
                            .arg(card.modelData.first).arg(card.modelData.last)
                            .arg(card.modelData.minutes.toFixed(1))
                            .arg(card.modelData.rate.toFixed(1))
                            .arg(card.modelData.model)
                    }

                    Kirigami.InlineMessage {
                        Layout.fillWidth: true
                        visible: card.modelData.link_ratio > 0.5
                        type: Kirigami.MessageType.Information
                        text: qsTr("Most posts share a link card: this may be people sharing the same headline.")
                    }

                    Flow {
                        Layout.fillWidth: true
                        spacing: Kirigami.Units.smallSpacing
                        Repeater {
                            model: card.modelData.links
                            delegate: Controls.Button {
                                required property string modelData
                                required property int index
                                text: qsTr("Post %1").arg(index + 1)
                                icon.name: "internet-services"
                                flat: true
                                onClicked: Qt.openUrlExternally(modelData)
                            }
                        }
                    }
                }
            }
        }
    }
}
