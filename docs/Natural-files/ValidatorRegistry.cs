namespace RuleGenerator.Validators;

/// <summary>Central validator registry. Generated — do not edit.</summary>
public static class ValidatorRegistry
{
    // CreditNote
    public static readonly CreditNoteCreateValidator CreditNoteCreate = new();

    // Discount
    public static readonly DiscountCreateValidator DiscountCreate = new();

    // Invoice
    public static readonly InvoiceCreateValidator InvoiceCreate = new();
    public static readonly InvoiceIssueValidator InvoiceIssue = new();
    public static readonly InvoiceVoidValidator InvoiceVoid = new();

    // Order
    public static readonly OrderApproveValidator OrderApprove = new();
    public static readonly OrderCancelValidator OrderCancel = new();
    public static readonly OrderCreateValidator OrderCreate = new();
    public static readonly OrderReturnValidator OrderReturn = new();
    public static readonly OrderSet_priorityValidator OrderSet_priority = new();
    public static readonly OrderSubmitValidator OrderSubmit = new();

    // OrderLine
    public static readonly OrderLineApply_discountValidator OrderLineApply_discount = new();
    public static readonly OrderLineCreateValidator OrderLineCreate = new();
    public static readonly OrderLineUpdateValidator OrderLineUpdate = new();

    // Payment
    public static readonly PaymentCreateValidator PaymentCreate = new();

    // Shipment
    public static readonly ShipmentCreateValidator ShipmentCreate = new();
    public static readonly ShipmentDispatchValidator ShipmentDispatch = new();

    // ShipmentLine
    public static readonly ShipmentLineCreateValidator ShipmentLineCreate = new();

    /// <summary>Runtime dispatch by entity and trigger name.</summary>
    public static void Validate<TContext>(string entity, string trigger, TContext context)
        where TContext : class
    {
        var key = $"{entity}.{trigger}";
        switch (key)
        {
            case "CreditNote.create":
                if (context is CreditNoteContext creditnoteCtx)
                    CreditNoteCreate.Validate(creditnoteCtx);
                break;
            case "Discount.create":
                if (context is DiscountContext discountCtx)
                    DiscountCreate.Validate(discountCtx);
                break;
            case "Invoice.create":
                if (context is InvoiceContext invoiceCtx)
                    InvoiceCreate.Validate(invoiceCtx);
                break;
            case "Invoice.issue":
                if (context is InvoiceContext invoiceCtx)
                    InvoiceIssue.Validate(invoiceCtx);
                break;
            case "Invoice.void":
                if (context is InvoiceContext invoiceCtx)
                    InvoiceVoid.Validate(invoiceCtx);
                break;
            case "Order.approve":
                if (context is OrderContext orderCtx)
                    OrderApprove.Validate(orderCtx);
                break;
            case "Order.cancel":
                if (context is OrderContext orderCtx)
                    OrderCancel.Validate(orderCtx);
                break;
            case "Order.create":
                if (context is OrderContext orderCtx)
                    OrderCreate.Validate(orderCtx);
                break;
            case "Order.return":
                if (context is OrderContext orderCtx)
                    OrderReturn.Validate(orderCtx);
                break;
            case "Order.set_priority":
                if (context is OrderContext orderCtx)
                    OrderSet_priority.Validate(orderCtx);
                break;
            case "Order.submit":
                if (context is OrderContext orderCtx)
                    OrderSubmit.Validate(orderCtx);
                break;
            case "OrderLine.apply_discount":
                if (context is OrderLineContext orderlineCtx)
                    OrderLineApply_discount.Validate(orderlineCtx);
                break;
            case "OrderLine.create":
                if (context is OrderLineContext orderlineCtx)
                    OrderLineCreate.Validate(orderlineCtx);
                break;
            case "OrderLine.update":
                if (context is OrderLineContext orderlineCtx)
                    OrderLineUpdate.Validate(orderlineCtx);
                break;
            case "Payment.create":
                if (context is PaymentContext paymentCtx)
                    PaymentCreate.Validate(paymentCtx);
                break;
            case "Shipment.create":
                if (context is ShipmentContext shipmentCtx)
                    ShipmentCreate.Validate(shipmentCtx);
                break;
            case "Shipment.dispatch":
                if (context is ShipmentContext shipmentCtx)
                    ShipmentDispatch.Validate(shipmentCtx);
                break;
            case "ShipmentLine.create":
                if (context is ShipmentLineContext shipmentlineCtx)
                    ShipmentLineCreate.Validate(shipmentlineCtx);
                break;
            default:
                throw new ArgumentException(
                    $"No validator registered for '{entity}.{trigger}'");
        }
    }
}