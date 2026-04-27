using System.Windows;
using System.Windows.Controls;
using System.Windows.Markup;

namespace IrisChat.Chrome;

[ContentProperty(nameof(Children))]
public partial class SectionCard : UserControl
{
    public SectionCard()
    {
        InitializeComponent();
    }

    public UIElementCollection Children => ContentHost.Children;
}
