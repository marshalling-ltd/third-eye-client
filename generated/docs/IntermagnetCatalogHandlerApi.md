# \IntermagnetCatalogHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**get_intermagnet_catalog_handler**](IntermagnetCatalogHandlerApi.md#get_intermagnet_catalog_handler) | **GET** /api/v1/intermagnet-catalogs/{id} | 
[**list_intermagnet_catalog_handler**](IntermagnetCatalogHandlerApi.md#list_intermagnet_catalog_handler) | **GET** /api/v1/intermagnet-catalogs | 



## get_intermagnet_catalog_handler

> models::IntermagnetCatalogRestModel get_intermagnet_catalog_handler(id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**id** | **uuid::Uuid** | Id for fetching intermagnet catalog | [required] |

### Return type

[**models::IntermagnetCatalogRestModel**](IntermagnetCatalogRestModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## list_intermagnet_catalog_handler

> models::PagedResponseIntermagnetCatalogRestModel list_intermagnet_catalog_handler(page, limit, order_by, desc, location_id, publication_state, cadence, orientation, has_scheduled_job, has_data, include_data_availability)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**page** | Option<**i32**> | page number |  |
**limit** | Option<**i32**> | page size |  |
**order_by** | Option<**String**> | list order by |  |
**desc** | Option<**bool**> | should use descending instead |  |
**location_id** | Option<**uuid::Uuid**> | intermagnet location id |  |
**publication_state** | Option<[**IntermagnetPublicationState**](.md)> | publication state |  |
**cadence** | Option<[**IntermagnetCadence**](.md)> | cadence |  |
**orientation** | Option<[**IntermagnetOrientation**](.md)> | orientation |  |
**has_scheduled_job** | Option<**bool**> | Filter by catalogs that have a scheduled job. If true, only catalogs with a scheduled job are returned. If false, only catalogs without a scheduled job are returned. If not specified, all catalogs are returned regardless of scheduled job status. |  |
**has_data** | Option<**bool**> | Filter by catalogs for which data has been downloaded. If true, only catalogs with downloaded data are returned. If false, only catalogs without downloaded data are returned. If not specified, all catalogs are returned. |  |
**include_data_availability** | Option<**bool**> | Include data availability time range from Intermagnet server in the response. If true, includes the start and end time for data availability for each catalog. If false or not specified, data availability information is excluded from the response. |  |

### Return type

[**models::PagedResponseIntermagnetCatalogRestModel**](PagedResponse_IntermagnetCatalogRestModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

